# Compass compatibility

Compass is an independent native product. Its compatibility contract is defined
by the shipped `compass` CLI, documented file and protocol formats, native tests,
and migration notes. Compass does not execute, import, check out, or test against
Graphify.

## Supported product identity

- executable: `compass`
- default artifact root: `compass-out/`
- project ignore file: `.compassignore`
- project configuration: `.compass/`
- environment variables: `COMPASS_*`
- MCP server and resources: `compass` and `compass://...`

Legacy Graphify executable, environment, configuration, and protocol names are
intentionally unsupported. Existing `graphify-out/` state can remain in place
while a fresh Compass build creates `compass-out/`; the two products do not
share caches or mutable state. See [`MIGRATION.md`](MIGRATION.md) for the
transition procedure.

## VS Code extension compatibility

The Compass VS Code extension requires Compass CLI 0.3.0 or newer. Releases
below 0.3.0 and 0.3.0 prereleases are unsupported even when they advertise an
individual feature or contract used by the extension. The extension reports
the minimum-version failure before activating repository workflows; it does
not maintain command-specific fallbacks for older releases.

Compass 0.3.0 itself remains supported. The extension adapts typed call-query
results for the known nested-anchor limitation in that stable release.

## Graph cache identity and MCP request snapshots

Graph query, impact and traversal caches now bind to the SHA-256 of the bounded
JSON bytes read, including when file size and modification time are unchanged.
Their disposable header versions advance to `TRAILG02`, `TRAILA03` and
`TRAILT07`. Typed content caches use `.content-v2.cache` and `CGRPHV02`; their
cache key, schema admission and decoded document come from one byte snapshot.
Older cache formats are ignored and rebuilt from the graph.

For standalone JSON graph paths, each MCP request checks current graph content
and holds an immutable context. Communities, hubs, neighbors, paths and typed
query identities within that context derive from the same captured bytes. A
later request observes a replacement or reports a missing, corrupt, unsupported
or oversized graph. A request already holding a valid context may finish using
that context after the path changes. Published SQLite queries retain their
existing store-reference validation path. Sidecars retain their own existing
loading policies; this graph snapshot does not make arbitrary external writes
transactional. Publishers should use atomic replacement.

Graph schemas, source identities, query-result schemas, historical realizations
and AST cache semantics are unchanged. Content reads use the configured graph
byte cap; this correctness correction makes no latency or memory improvement
claim.

## Go struct field declarations

The universal Go producer now publishes one `field` declaration per explicitly
named direct struct field, with its identifier source range and a directed
`contains` relation from the owning `struct`. A declaration with multiple
names creates distinct fields. Embedded fields, blank identifiers, and fields
inside anonymous nested structs do not become direct named members of the
outer struct. Existing type-reference and receiver-binding evidence remains.
This adds graph nodes and containment edges, so graph digests, communities,
hub degrees and hub member counts can change after a rebuild. It does not
assert field-access target resolution or god-object classification. Disposable
AST cache semantics advance from 11 to 12; old cache entries are ignored and
recomputed. Graph schema and package version remain unchanged.

## Go field-access evidence

The Go producer emits source-anchored `MemberAccess` occurrences for selector
expressions outside direct call targets. When bounded receiver typing establishes
an exact owner, it also emits an `AccessesMember` candidate restricted to a
source field declaration with the same qualified owner and name. Resolution
looks up that field before following its type alias; missing or ambiguous fields
remain unresolved. The graph projects resolved contacts as directed `references`
with exact occurrence provenance. These records do not claim read/write effects,
runtime aliases, promoted members or complete field-use coverage.

Rebuild graphs to see the additive references. Disposable AST cache semantics
advance from 12 to 13; old entries are ignored and recomputed. Existing graph
and evidence schemas and package version remain unchanged. Added references can
change degrees, navigation, clusters and communities. Historical realizations
remain immutable.

## Java field-access evidence

Java emits `MemberAccess` occurrences for ordinary fields and enum constants,
including supported unqualified references. A bounded AST lexical index distinguishes
parameters, locals, block/loop lifetimes, lambda/catch/resource bindings and
source type names. Receiver typing supports declared nominal values, source-local
field chains, arrays, casts, direct constructors and single generic bounds.
Simple `instanceof` branches and abrupt guards retain their flow scope. Visible
source types take precedence over imports and package prefixes. A qualified
field lookup requires one receiver type declaration; field availability cannot
select between duplicate nominal types. Exact declaration evidence remains
authoritative.

For an already established Java nominal receiver, resolution joins dotted
source type names to canonical nested declarations using the complete qualified
name. Imported nested types can therefore supply field and enum-member targets.
Package/type spelling collisions and duplicate enclosing types retain ambiguity;
member availability cannot select an owner. Nominal lookup work shares the
existing candidate budget. This does not infer a type from an arbitrary expression
chain. Rebuild stored graphs for the resolver correction; existing version-10
extraction facts remain usable because the emitted evidence is unchanged.

Unknown receivers, ambiguous names, unsupported pattern flow, inherited fields,
unregistered local/anonymous class owners and exhausted inference remain
unresolved or unrepresented. Unsupported flow masks a possibly shadowed field;
it never establishes a convenient outer-field target. Cross-file field chains
and general hierarchy/accessibility/type-checking are not inferred. Existing
qualified universal resolution selects field or enum-member declarations. Bare
enum switch labels use the selector type and permit only enum-member targets;
unknown selectors remain unresolved. Registered constant-specific bodies retain
their method/field owners and field shadowing. Their body-specific fields are
visible only through lexical ownership, not through an enum-typed value.
Type parameters and local values/types shadow same-named enum receivers. Graph v1 emits
`references` with exact member-access provenance, preserving occurrences without
read/write or runtime-alias claims. Method names are not field accesses.

Rebuild graphs to obtain these facts. AST cache semantics advance from 9 to 10;
evidence/graph schemas, producer capabilities and package version are unchanged.
Historical realizations remain immutable. Additional references can change
navigation and community assignments; neither implies improved community
quality or god-object classification.

## Rust field-access evidence

Rust extraction emits member-access occurrences for explicit field expressions,
including fields in method receivers and scalar-indexed receiver chains when
bounded source-type evidence establishes the nominal owner. Existing universal
resolution selects only field declarations, preserving parallel occurrences.
Graph v1 publishes these as `references` with member-access provenance, not as
read/write-effect or runtime-alias proofs. Method-selector syntax is excluded.

Unknown or shadowed receivers, ambiguous type paths, unsupported expressions,
raw pointers and exhausted receiver-depth inference remain unresolved. These
facts do not establish complete field-use coverage, class cohesion or god-object
defects. Rebuild graphs to obtain the additive access records. AST cache semantics
advance from 7 to 8; evidence/graph schemas, existing producer capabilities and
package version are unchanged. Historical realizations remain immutable.

## Rust indexed method receivers

Rust call extraction follows bounded field and scalar-index receiver syntax
when source types establish a standard `Vec`, array, or slice and a decimal integer
literal or `usize` index. Supported reference and standard `Box`/`Rc`/`Arc`
wrappers preserve the element type. Field types retain their declaration scope,
and repeated calls retain distinct source anchors. Type-path qualification
retains every intermediate module when expanding a local or imported alias.
Explicit standard-vector
and element imports may be resolved by existing import evidence.

Custom `Index` implementations, range indexes, unknown index types, raw
pointers, ambiguous or shadowed type names, and unsupported expressions do not
establish an element-method target. Prelude-vector inference is disabled by
visible wildcard imports or source attributes that may disable the standard
prelude. Generic field substitution and cross-file field layout discovery are
not compiler inference capabilities of this rule.

Disposable AST cache semantics advance from 6 to 7 for these extraction facts.
Rebuild a graph to obtain them; published history, evidence schema, producer
capabilities, graph schema, and package version remain unchanged.

## Compatibility evidence

Compass changes are verified with native evidence:

```bash
sh scripts/check_product_boundary.sh
cargo fmt --all -- --check
cargo clippy --workspace --lib --bins --locked -- -D warnings
cargo test --workspace --lib --bins --locked
cargo test -p compass-cli --test compass_product --locked
sh scripts/test_release_scripts.sh
cargo package --workspace --locked --no-verify
```

CI covers Linux, macOS, and Windows targets listed in
`.github/workflows/compass-ci.yml`. Release packaging, security hardening, and
performance checks are owned by Compass workflows and require no external
product checkout.

## Document and OCR compatibility

Native PDF, DOCX, PPTX, and XLSX processing is part of the local Rust product
boundary. It does not require Python, Tesseract, LibreOffice, Poppler, Java, a
runtime grammar download, or provider credentials. The stable artifact majors
introduced here are `compass.document/1`, `compass.document.inspect/1`, and
`compass.ocr/1`; unknown majors and normalizer versions fail explicitly.
The additive `compass.document.preview/1` payload is carried by Office
artifacts and the optional `document.previews` field in
`compass.viewer.graph/1`. It contains bounded, deterministic SVG snapshots for
DOCX normalized pages, PPTX slides, and XLSX sheet windows. Snapshots are
self-contained (only escaped text and data-URI PNG thumbnails), digest-bound,
and validated against script, external-resource, size, and geometry limits;
older readers may ignore them. They are an inspectable presentation surface,
not a claim of pixel-perfect Office layout, and native blocks remain
authoritative.

OCR is off by default. Enabling `auto` or `always` requires one exact verified
Compass-managed profile. Extraction never downloads models, and `models
verify` never accesses the network. Document and semantic caches are hard-cut
by source digest, schema, normalizer, rasterizer, OCR policy, preprocessing,
profile manifest/model digests, and languages. An incompatible cache entry is
a miss or explicit corruption error, never a fallback to flattened text.

Managed OCR is unavailable on Intel (`x86_64`) macOS because the pinned ONNX
Runtime distribution has no self-contained build for that target. Compass
therefore omits the OCR runtime dependency on Intel macOS instead of requiring
a system ONNX installation. Native document processing and `--ocr off` remain
fully available; `models install` and OCR-enabled processing fail explicitly.

The selected OCR identity is included in graph build and immutable history
profiles. Native text remains authoritative; OCR is additive derived evidence
with exact source owner, geometry, confidence, and model provenance. Partial
visual coverage is never labeled complete or finalized as a complete document
cache entry.

Semantic enrichment accepts explicit provider/model selection through
`--backend`/`--model` or the non-secret `COMPASS_BACKEND`/`COMPASS_MODEL`
environment defaults. Provider credentials remain provider-specific
environment or secret-store values and are excluded from graph artifacts,
history profiles, and cache identities.

## Evolving contracts

### Owner-qualified symbol lookup

Typed callers, callees, impact and node-trail queries now accept a unique
owner/member suffix such as `APIRoute.get_route_handler` when the stored
qualified name is `fastapi.routing.APIRoute::get_route_handler`. An exact ID or
full normalized name still takes precedence. The lookup reads the bounded
exact leaf-name posting and verifies every candidate's full owner suffix;
multiple matches or an exhausted candidate bound remain ambiguous. If the
leaf name exists but the owner does not, no fuzzy winner is chosen. Existing
fuzzy fallback remains available for a misspelled leaf with no exact leaf-name
candidate. JSON response schema and graph artifacts are unchanged; previously
fuzzy or unresolved typed queries may now return a source-identifiable direct
answer. This behavior applies to `ask` when it routes to those typed queries.
Legacy `explain` and `path` also accept the unique owner suffix after checking
exact IDs and full names. Their existing candidate-list behavior preserves
multiple matches. No graph rebuild or package-version change is required.

### MCP paths

`shortest_path` resolves exact node IDs or normalized symbol/qualified names.
It no longer substitutes a scored fuzzy endpoint. Ambiguous matches return at
most 20 candidates, ordered by ID, with an omission count; callers choose an
exact ID before requesting a path. Both endpoints resolving to the same node
continues to produce a diagnostic rather than a positive path.

Search remains undirected and minimum-hop. Among equal-hop routes it now uses
the query engine's structural relation costs and deterministic path keys.
`max_hops` is enforced during traversal, accepts 0–64, and defaults to 8.
The same 1,000,000-adjacency-entry and 16 MiB cumulative path-key budgets as
the CLI path engine apply. Exhaustion fails explicitly. A depth-limited miss
does not assert global disconnection or the length of an unsearched route.

Uniquely resolved, distinct endpoints add `compass.mcp.path/1` in the existing
structured transport envelope. It distinguishes `found`, `depth_limit`, and
`disconnected` and retains exact ordered node/edge identities. Resolution
diagnostics retain their text form. Legacy text is still available, including
through the non-transport `invoke` helper. Graph schemas are unchanged.

### MCP hub identities

Each hub now also carries an additive `connectivity` object with schema
`compass.hub-connectivity/1`, and text includes its node kind and relation
breakdown. Hub eligibility, degree, and ranking are unchanged. Incident record
counts retain parallel records; they are not distinct-neighbor degree or
independently verified source occurrences. At most 16 relation categories are
shown, ordered by record count then name, with explicit omission totals.
Undirected artifacts do not acquire inferred directions in this summary.
Their `ranking` metadata now correctly says
`distinct-undirected-endpoint-degree`; directed artifacts retain
`distinct-directed-endpoint-degree`.

MCP `god_nodes` adds `structuredContent` using the existing
`compass.mcp.tool-result/1` envelope and the result schema
`compass.mcp.hubs/1`. The ranked records preserve exact IDs, kinds, degrees,
and available source anchors. Text and the `compass://god-nodes` resource add
an ID/source/location line beneath each entry; string values are JSON-escaped
so IDs remain recoverable without injecting extra lines. Missing anchors are
null. Inputs, eligibility, degree calculation, and ranking are unchanged.
An additive `memberEvidence` object with schema `compass.hub-members/1` now
appears for directed typed class and struct hubs. It counts uniquely owned
direct methods/fields, ambiguous direct members, and stored method-to-own-field
`references` records, pairs and participating methods. It is null for other
kinds and undirected graphs. Parallel records remain in the record count;
ambiguous owners cannot contribute to own-field counts. These observations
include stored reference records regardless of their confidence and carry
`sourceCoverage: "unverified"`; they are neither validated reference targets,
complete source-access coverage nor a god-object classification.
Exact node lookup checks the original ID before the existing whitespace-trimmed
fallback, preserving distinct legacy IDs that differ by surrounding whitespace.
The non-transport `CompassMcp::invoke` compatibility helper still returns text
for this tool. Machine consumers should use the structured projection rather
than parse display labels. See [output contracts](docs/reference/outputs.md#mcp-hub-results).

### MCP community and neighbor lookup

The disposable traversal cache now retains a labeled community's numeric ID
as well as its name. Cache magic advances from `TRAILT04` to `TRAILT05` so old
projections rebuild from their unchanged graph. MCP membership, statistics,
and other traversal consumers can now observe those stored communities.
Graph schemas and published historical graphs are unchanged.

MCP `get_neighbors` first resolves an exact ID or normalized symbol/qualified
name using the shared exact lookup, including its evidence-gated export-binding
handling. Prefix/substring fallback applies only when no exact candidate exists.
A unique exact `term_len()` therefore remains navigable when `test_term_len()`
exists. Multiple exact candidates remain ambiguous; ranking never chooses one.
The ambiguity list is ordered by exact ID with at most 20 displayed candidates
and an omission count. Retry with an exact ID to choose a declaration. Exact IDs
retain their case. The tool's input schema and the MCP result envelope are
unchanged. This corrects unnecessarily ambiguous lookups; no migration is needed.
Relationship filters apply before repeated neighbors are grouped, so a stored
call remains visible when a containment/reference edge precedes it. The tool
continues to display distinct neighbors. Successful results now add the versioned
`compass.query.neighbors/1` structured result inside the existing MCP transport
envelope. Each direction/neighbor group carries its exact node record and all
matching relationship records, including IDs, source sites, provenance and
parallel occurrences. Missing legacy edge IDs remain absent. Directions describe
stored endpoints; `graphDirected` preserves the artifact metadata, so an
undirected artifact does not become proof of a directed call. Text adds escaped
ID/source/location lines; the displayed relation represents the first canonical
record in the group. Use structured records for all relations and occurrences.

Neighbor lookup reads one full snapshot to avoid losing fields in the compact
traversal cache. It is bounded by 1,000,000 examined adjacency entries, 10,000
matching incident records, a 4,096-byte filter and a 1 MiB structured result.
Exhaustion is an explicit error, never an empty or silently partial success.
Groups sort by outgoing/incoming direction then exact ID; records sort by
canonical JSON. Self loops appear in both directions and count once against
the incident-record budget. Published graphs and historical artifacts are unchanged.

### Calls-only directed trails

`node --calls-only`, MCP `get_node` with `calls_only: true`, and explicit
`ask "call path from SOURCE to TARGET"` / `"call chain from SOURCE to TARGET"`
restrict trails to stored directed `calls` edges. Structural shortcuts cannot
satisfy this request. The typed `NodeTrailRequest` adds optional boolean
`callsOnly`; omission defaults to false and false is omitted when serialized.
Rust struct-literal callers must initialize the new `calls_only` field.

The policy applies to endpoint relationship-role probes, traversal, depth
frontiers and undirected direction diagnostics. Exact-name ambiguity remains
unresolved. Existing heuristic opt-in, occurrence provenance, deterministic
ties, node/edge/byte bounds and partial-graph diagnostics remain effective.
The undirected diagnostic charges examined structural records to its work
budget even though they cannot form a call path. Empty bounded results remain
incomplete, never proof of no call path.

Existing structural trail and natural `path from` requests retain their
behavior. The response stays `compass.query/1`; Agent View request metadata
records the explicit call-path question. No graph, extraction/cache, history,
or package version changes are needed. These are static call relationships,
not a guarantee that the chain executes for every input.

### Directed trail depth diagnostics

Typed `node`/`get_node` trails now report `truncated: true` with
`bounded_truncation` when an unsuccessful search reaches its depth bound and
cannot prove the reachable frontier is closed. They do not use a shorter
undirected route to assert a global direction mismatch while an unexplored
forward continuation remains. Frontier checks share the declared work budget
and run only after the bounded path search fails, preserving positive paths
within the requested depth. Closed dead ends and cycles can still yield complete
negative results. The `compass.query/1` schema is unchanged; consumers should
continue to distinguish incomplete searches from negative answers.

`explore` / `explore_code` also retain incomplete-search status when no connecting
path was found; previously that status could be lost with the absent path.

### Bounded node trails

The undirected `path` command also retains nondominated cost/depth states.
Each weighted or alternative search is bounded to 1,000,000 adjacency entries
and 16 MiB of cumulative path-key bytes. Exceeding either returns a nonzero
work-limit error, never `NO PATH FOUND`. Previously expensive requests may now
need a smaller depth or graph. Relation weights and output schemas are unchanged.

Typed `node`/node-trail queries keep nondominated arrivals by node and depth,
so a cheaper but longer prefix cannot hide a valid trail within `max_depth`.
Rejected nodes are not considered admitted on a later visit. Existing cost
weights, direction, deterministic tie rules, work limits and response schema
remain unchanged. Previously missing trails can now be returned; incomplete
responses no longer include nodes admitted after their budget was exhausted.

### Hub ranking

New god-node analyses order equal-degree candidates by stable node ID instead
of input record order. Source-located declarations named `Path`, `Counter`,
`Enum`, or other names also used by libraries are no longer suppressed by name
alone. Explicit canonical node kinds take precedence over display-label
heuristics throughout topology analysis: a `.method()` label does not turn a
method into a file, and file nodes remain excluded even with descriptive labels.
Typed structural nodes with nonempty source paths are not classified as concepts
merely because their source filename has no extension.
Legacy records without a recognized kind retain the existing label fallback.
Isolated declarations are omitted. The serialized `id`, `label`, and
`degree` fields and degree calculation are unchanged; the candidate list can
change. Published historical artifacts are not rewritten. Hub rank describes
connectivity, not a verified god-object design defect.


Rust structural evidence now uses producer version 2. The evidence and graph
schema majors are unchanged, but Rust extraction caches from producer version
1 are rebuilt so source-proven standard-library dereference chains and
evaluating local macro inputs can publish newly recovered exact calls.
Unsupported macro shapes, non-evaluating inputs, and ambiguous receiver owners
remain unresolved rather than being guessed.

Rust receiver lookup now respects local shadowing in lets, loops, closures,
match arms/guards, and conditional lets. An unknown inner type cannot inherit
an outer parameter's alias. A let initializer still sees the previous binding;
an `else` branch does not see bindings from a failed condition. Previously
incorrect call edges can disappear and remain unresolved until the receiver
type is proven. The advertised producer capabilities and evidence/graph schemas
are unchanged. AST cache semantics advance from 2 to 3, rebuilding prior AST
facts automatically across languages. Published historical graphs are unchanged.

Rust extraction now follows a bounded source-proven `Result<Vec<_>>` collected
from a standard vector iterator and a constructor declared to return standard
`Result<Self>`. Its `Ok` match binding, vector loop variables, and `iter().map`
closure parameters can gain exact method-call targets. Local aliases and
unsupported iterator or constructor forms remain unresolved. AST cache
semantics advance from 10 to 11, rebuilding disposable facts; graph and
evidence schema majors and published historical graphs are unchanged.

Go receiver lookup now includes `if` and switch initializers alongside loop
initializers. Block locals, range variables, and closure parameters are resolved
in lexical order; an unsupported nearer binding cannot inherit an outer
parameter's receiver type. Local callbacks and shadowed package names cannot
provide a same-named global factory's return type. Type-switch aliases block
outer types but do not yet infer case-specific narrowing. Invoking a returned
callback retains the inner factory call without treating the outer invocation
as a reference to the factory receiver's type. Newly recovered and
corrected call edges require rebuilding the graph. AST cache semantics advance
from 3 to 4, invalidating older disposable AST facts across languages. Producer
capabilities and evidence/graph schemas are unchanged; published history remains
immutable.

Java method receivers now use constructor syntax evidence for direct named
object creation, including qualified type names and bounded parentheses.
Overload selection still requires the existing argument evidence. Anonymous
subclasses, explicit enclosing-instance creation, arrays, casts, and chained
results retain unresolved method candidates when their receiver ownership is
not proven. Arbitrary expression text no longer becomes an external type name.
An enclosing-instance construction also retains its receiver qualifier instead
of selecting a same-named imported class. AST cache semantics advance from 4
to 5, rebuilding older disposable facts across languages and invalidating old
build seals. Producer capabilities, graph/evidence schemas, and published
historical realizations are unchanged.

Java varargs signatures now retain their spread parameter, and canonical
parameter types represent it as an array. Explicit array arguments and trailing
parameter dimensions preserve their rank. Supported overloads are considered
in strict fixed-arity, loose fixed-arity, then variable-arity phases. Missing
argument/hierarchy evidence and incomparable overloads remain unresolved;
unequal variadic prefixes do not establish a most-specific target. Rebuilding
can change signature metadata and call targets. AST
cache semantics advance from 5 to 6; published history, producer capabilities,
and graph/evidence schemas remain unchanged.

### Framework route hierarchy

Framework route hierarchy now requires a recognized filesystem-convention fact
from its owning framework producer. A receiver name such as `r` or `app` does
not establish parentage between programmatic routes in separate source files.
Framework composition rules still own programmatic mounts and groups. The
framework-pack semantics identity initially advanced from 6 to 7, and build-state seals
now include that identity. Rebuild existing graphs to remove unsupported
containment edges and recompute affected paths, degrees, and communities.
Graph/evidence schema majors and immutable historical realizations are unchanged.

Framework semantics 8 additionally replaces first-file directory selection with
framework-specific nesting. Next App Router parents must be layout modules;
flat route parents use filename segments, including pathless and non-nesting
markers; Nuxt parents require a matching page module above a child directory.
Index pages and sibling modules cannot become parents merely through ordering.
Ambiguous nearest parents remain unresolved instead of falling back outward.
Rebuild graphs to correct containment, degrees, paths, and communities. These
rules do not infer custom router configuration, runtime mounts, or an omitted
layout declaration. See the supported boundaries in the
[framework graph reference](docs/reference/react-framework-graph.md).

### Agent Query View

Compass adds the additive strict projection `compass.query.agent-view/1` for
typed CLI and MCP consumers. It is derived from, and digest-bound to, the raw
`compass.query/1` or `compass.query.discovery/1` response. The raw CLI `json`
shape, MCP `structuredContent.result`, graph schemas, and discovery
`compass.query.discovery-text-page/2` cursor meaning are unchanged.

The typed commands accept `--format agent-json`; default text is an
answer-first presentation. MCP keeps `compass.mcp.tool-result/1` and adds the
optional `agentView` sibling plus a code-query `semanticResultDigest`. Existing
consumers may ignore the optional projection. Consumers that consume Agent
View must reject unknown major versions, enforce the documented bounds, and
distinguish `no_match`, `needs_resolution`, `no_path`, source truncation, and
projection truncation from a positive complete answer.

The additive `relationship_inconsistency` diagnostic extends the strict
`compass.query/1` diagnostic enum and changes its contract fingerprint. Strict
TypeScript consumers and the checked-in manifest must accept the new value
before interpreting a relationship result that carries it.

### Typed text pages and store self-check

Typed commands (`ask`, `search`, `callers`, `callees`, `impact`, `explore`, and
`node`) accept `--text-budget` and `--cursor` for `--format text`. Text output
is now paged with the additive `compass.query.agent-text-page/1` cursor: the
page carries a `Pagination:` footer, and the cursor continues the same
deterministic ledger at the same page budget. The ledger is derived from the
raw `compass.query/1` response, so a page can reach records that the compact
`compass.query.agent-view/1` bounds omit. The raw `json` and `agent-json`
shapes are unchanged, and a cursor is rejected for non-text formats. Cursors
remain valid only for the same operation, graph identity, and reviewed prefix;
consumers must treat an invalid cursor as an explicit failure rather than
falling back to the first page.

`compass store validate` now materializes the selected snapshot and applies
the strict `compass.graph/1` validation used by readers, in addition to the
existing tree-integrity, manifest, and `store.ref` checks. An artifact that an
older publisher wrote with records that strict readers reject now reports
`valid: false` with the offending record IDs; rebuild the graph (or restore a
validated backup) instead of querying it. `compass store status` intentionally
keeps the cheaper digest-and-integrity check and does not claim semantic
validation.

Continuation pages of the discovery text pager (`compass query --cursor`) and
of the typed agent text pager now replace the repeated caveat block with a
single `CAVEATS: N unchanged from page 1 (code×count)` line. Page one still
prints every caveat in full, the cursor contract and pagination footer are
unchanged, and the same page budget now carries more result entries. Consumers
that parsed caveat text from continuation pages must read it from page one.

### TypeScript path aliases and file-shaped path input

Exact name lookup can identify both an export binding and its declaration.
When complete, bounded graph evidence proves that the binding's exact source
range belongs to one owner and exports one matching declaration, lookup removes
the redundant binding candidate. Both records retain their graph identities;
an exact export ID still selects the binding. The declaration must already be
in the exact-name candidate set and share the binding's qualified name,
normalized name, and source file. This applies to typed relationship/trail
queries and the exact lookup used by paths, explanations, and MCP navigation.

Different declarations, multiple export targets, incomplete source ranges,
inferred/ambiguous/deferred evidence, and incomplete candidate sets keep their
ambiguity. Additional proof examines at most 256 candidates and 1,024 adjacency
entries plus a truncation probe. Exhaustion retains the original candidates;
typed responses report truncation and an ambiguity diagnostic. Graph schemas,
stored identities, and extraction are unchanged. Existing graphs can receive
this lookup correction without re-extraction.

Traversal cache format `TRAILT07` retains deferred relationship flags and the
weakest confidence across all evidence (introduced in `TRAILT06`), including
explicit compatibility confidence. Missing or unknown confidence values in an
evidence item cannot establish an exact fact. Older disposable traversal
caches rebuild from the authoritative graph; published historical graphs are
not rewritten.

A TypeScript or JavaScript project that is the `extends` base of another
project keeps its own `compilerOptions.paths` when it declares `files` or
`include`. Only a base config without its own file set stays excluded, and a
same-directory project that extends another config still takes precedence over
the config it extends. Graphs for such repositories therefore gain import,
export, reference, and call edges that were previously missing; rebuild the
graph to publish them. Unchanged behavior is preserved for same-depth configs
that both own an importer: resolution still fails closed rather than guessing.

`compass path` resolves a file-shaped endpoint to the file's content node when
the file node itself carries no relationships and exactly one module node owns
the same source file. The answer names the module and its source file, so the
endpoint remains identifiable. Languages that already connect their file nodes
(for example Go, Python, and Rust) keep the existing behavior, and an ambiguous
or multi-module file still fails closed.

When a graph publishes no separate `file` node, a repository-relative source
path now resolves through the source-backed node index: one module owner is
selected when unique, while multiple modules or declarations remain an explicit
ambiguity. This lets path traversal follow the same stored workspace import
edges used by `callers`.

`compass explain` includes a bounded source excerpt by default for a uniquely
resolved source-backed declaration. When a stored symbol digest matches the
complete recorded byte span, the excerpt is labeled `digest-verified`. A graph
without that digest can still supply a current excerpt, explicitly labeled
`unverified: no recorded source digest`; an anchor alone cannot establish
freshness. A mismatching or malformed digest prevents source output. This
corrects the earlier unconditional verification label, including text inside
shared JSON output envelopes. The query library exposes `digest_verified` on
`ExplainedSource`; machine schemas and graph artifacts are unchanged.
`--no-source` restores a metadata-only answer; `--source` remains accepted.
Ambiguous or unsourced targets do not produce source text.

### Exact symbol search

`search --exact` and MCP `search_symbols` with `exact: true` use the existing
bounded exact ID/name index without lexical or fuzzy recall. Optional CLI
`--file`, `--line`, `--kind` filters correspond to MCP `source_file`,
`start_line`, `kind`. Filters require exact mode; a line also requires a file.
File strings match stored paths exactly and are not opened or canonicalized.
Selectors and file filters accept 1..4096 non-control bytes; lines are positive
32-bit integers and kinds use the stored node-kind spellings.

Exact IDs take precedence over name lookup, then must satisfy every supplied
filter. Names use the existing normalization (trim whitespace, trailing `()`,
leading `.`, and lowercase); file paths and IDs remain case-sensitive. All
matching records remain visible, including overloads and export bindings.
Scores are uniformly 1 and order is stable by ID. This mode does not select a
winner or collapse a binding into its declaration.

The candidate cap applies before filtering. A truncated prefix cannot prove
uniqueness or absence, even when filtering returns one or zero nodes. Node and
response bounds and coverage diagnostics still apply. Exact CLI text requests
do not automatically widen candidate bounds. Default ranked search and the
`compass.query/1` response schema remain unchanged. The query API adds
`search_exact(SearchRequest, ExactSearchFilter)` without changing `SearchRequest`.
Agent View uses typed name normalization when identifying exact search matches.
An empty bounded search without a no-match diagnostic retains `match=unknown`
and partial execution instead of asserting absence or failing view validation.

### Explanation member source

`explain --source-members` replaces the declaration excerpt with callable member
excerpts reached through outgoing recorded `contains` / `method` relationships.
It follows nested type containers, preserving recorded direction and parallel
membership evidence in the query library. It does not infer ownership from
names or file proximity, follow calls/references, or promote inferred/deferred
membership. Ambiguous roots remain unresolved and undirected graphs are refused.
Default `explain` behavior is unchanged; `--source-members` conflicts with
`--no-source`.

Members are ordered by source file, byte range, and exact ID. They share
`--max-source-bytes` (default 4096; maximum 1 MiB in member mode). Discovery has
separate limits of 128 callables, 128 containers, depth 4, 10,000 adjacency
entries and 1 MiB of metadata. Verification attempts share a 16 MiB recorded-span
budget. Discovery-bound failures report unavailable member source; source/work
exhaustion reports truncation and omitted member counts. Individual source
failures remain visible without suppressing other valid excerpts. Stored
source digests and containment checks use the existing source reader.

Optional `--member-focus TEXT` requires member mode. It ranks callables by the
number of distinct normalized focus terms in their recorded names, then by the
original source order. Existing query-term and identifier normalization apply;
input is limited to 4,096 bytes and 32 distinct searchable terms. Empty or
unsearchable focus is rejected. Duplicate terms do not add weight. No matches
preserve source order; unmatched members remain candidates. Source is not read
to rank, and focus does not resolve ambiguous roots or relax any existing limit.
The report names focus terms and each retained member's lexical matches. These
matches do not establish responsibility, behavior or semantic relevance.

This is an additive CLI option and query API, with no graph or shared-output
schema change. The text reports exact member IDs, source anchors, verification
status, retained source bytes, and unavailable/omitted counts. It is structural
source evidence, not a synthesized explanation or a god-object diagnosis.

### Typed query deadlines

`ask`, `search`, `callers`, `callees`, `impact`, `explore`, and `node` accept
`--timeout-ms <N>` with a default of 60000 and a hard maximum of 600000. The
deadline is armed once per command - a page continuation or bound widening
shares it - and is checked between resolution, candidate, relationship, impact,
and path-expansion steps. An expired deadline returns the typed
`code_query_timeout` failure with a hint to raise `--timeout-ms` or lower the
record bounds; partial results are not published. Library and MCP callers keep
the previous unbounded behavior unless they arm a deadline with
`CodeQueryEngine::with_deadline`, so no existing response shape changes.

### Review Markdown sections

`compass review --format markdown` accepts the additive `--section NAME`
(repeatable or comma-separated) and `--list-sections` options. Section names
are `summary`, `risk-factors`, `merge-checks`, `findings`, and `not-included`;
an unknown name is a usage error. `--list-sections` prints one name per line and
does not require a comparison. A filtered report always keeps the `## Compass
PR review` title and the report reference so the extracted text still
identifies the canonical report, and the unfiltered default output is
byte-identical to the previous rendering. `--max-findings` and
`--max-output-bytes` continue to bound the projection and report exact
omissions; `--section` and `--list-sections` are rejected with `--readiness` and
with non-Markdown formats.

### Compact Agent View

Typed commands accept the additive `--brief` flag with `--format agent-json`,
which emits `compass.query.agent-view.brief/1` instead of
`compass.query.agent-view/1`. The brief projection keeps the answer semantics
(`status.resultState`, `matchState`, `coverage`, headline, caveats,
source-located entities, relationships with relation/site/confidence, paths,
and next-action argv) and drops audit-only detail: graph and build identities,
result and view digests, omission counters, per-relationship IDs, per-entity
roles, and per-edge evidence layers. Consumers that need exact identity,
digests, or evidence read `--format json` (the unchanged raw
`compass.query/1` response) or omit `--brief`. `--brief` is rejected with any
other format, and the existing `agent-json` output is byte-identical to the
previous release.

### Agent View relationship ordering

`compass.query.agent-view/1` and `compass.query.agent-view.brief/1` now order
`relationships` by relation strength - direct usage first (calls, instantiation,
routing, handlers, registration), then imports/exports, then references and
documents, then everything else - with the exact relationship ID as the
deterministic tie-break. `primaryResults` for callers and callees follow the
same order, and the callers/callees headlines report the source response's edge
count rather than the capped projection count. The fields, schemas, digests,
and raw `compass.query/1` response are unchanged; consumers that relied on the
previous ID-sorted presentation must treat the new order as the contract.

### Impact traversal and result ordering

`compass impact` (and the compatibility `affected` command that shares the
reverse walk) visits edges that name the expanded node before edges that only
reach its containing owner, then orders by relation strength and the exact
edge ID. The retained trail ledger is capped, so visit order decides which
dependents survive a bounded response: a symbol with hundreds of owner-level
references no longer loses its direct callers. The agent view orders the
impacted nodes by trail length and the strength of the trail's last hop, so a
direct caller is reported before a symbol that only touches the containing
owner. `compass.query/1` keeps its schema, and the raw response now records the
direct evidence in `paths` where it previously held owner-level trails.

### Unresolved relationship headlines

`compass callers`, `compass callees` and `compass impact` answer about the
symbol the query resolved. When the query has no exact match, the Agent View
state is `no_match` and the headline now says so and names whose evidence the
rows are - "No exact match for \"PathRouter::route\"; the 27 incoming usage
relationship(s) below belong to the fallback candidate
axum::routing::Router::route_layer." - instead of reporting a count "for" a
fallback candidate as if it answered the request. The candidate list stays in
the caveats, exactly resolved queries keep their previous wording, and no
schema, count, or omission field changes.

### Owner-level importer probe bound

`compass callers`, `compass callees` and `compass impact` recover evidence that
targets a containing owner (a module, file or alias target) through a
term-posting probe. Each candidate the probe verified cost one snapshot read,
so a symbol whose owner carries a common identifier could turn a two-edge
answer into ~1,000 reads: 7.7 s per `callers` query on the Axum corpus and 50 s
per `impact` query on Zod and Gson. The probe is now a fallback rather than an
always-on enrichment: it runs only while the containment walk has published
fewer than `RELATIONSHIP_SELF_CHECK_MIN_IMPORTERS` (8) edges, verifies at most
16 candidate sources even then, and keeps reporting the observed importer count
as a lower bound through `relationshipInconsistency`. The owner-scoped
adjacency still publishes every direct, module-level and alias-target edge it
resolves, so an answer that was complete at the caller's `--max-edges` bound
stays complete; an answer whose owner-level importer set is wider is bounded
rather than unbounded work, and a well-connected symbol's answer no longer
carries supplementary owner-level importer edges beyond that. The
`compass.query/1` schema, limits and diagnostics are unchanged.

### Agent text page budget

The paged text projection (`--format text`, and the `text` views of `search`,
`query`, `callers`, `callees`, `impact`, `explore`, and `node`) now spends its
budget on evidence instead of envelope. One page renders at most the profile the
Agent View already documents - 12 primary results, 24 relationships, 5 paths -
and reports the ledger's true total in `range=A-B of T`, so `next=` continues
the rest. The `RESULT` block is one line
(`RESULT <state> · match=… · evidence=… · execution=… · coverage=…`); the
pagination line drops the version and budget echo; the `Bound:` and
`Completeness:` lines print only the bounds that withheld records; blocking
caveats keep their full statement while warnings print their actionable
sentence, with the remainder in `agent-json`/`json`; and a `path` answer prints
its endpoints as labels without repeating their identifiers. A `PATHS` row in
the text view prints the hop count and the labelled chain; its path identity -
a concatenation of every node identifier on the trail - stays in
`agent-json`/`json`. The same footer rule applies to every other paged text
answer (`path`, `node`, `explore`): `Pagination: page=A/B <item>=X-Y/T next=…`
without a budget echo or a previous-page field. Stable entity identifiers are
printed only for a non-exact match; resolved answers and exact-name pick lists
print the qualified name and source anchor, and a relationship row states its
confidence and resolution only when they are not the strongest `exact`. The raw
`compass.query/1` response and the
`agent-json` view still carry every identifier, anchor, and omission counter.
Continuation cursors use a compact wire encoding that stores 64-bit digest
prefixes and a version byte; cursors issued by an earlier release are rejected
with an explicit version error instead of being reinterpreted. Consumers that
parsed the previous multi-line `State:`/`Match:` block or the version prefix of
the pagination line must read the new single-line form.

### Discovery agent-noun expansion

Natural discovery now adds graph-verified agent-noun spellings of behavior
terms to the bounded ranking terms: a term ending in a silent `e` also tries
`-er` and `-or` (`route` → `router`, `validate` → `validator`), and a variant is
kept only when the graph's bounded name index returns at least one node for it.
The response schema, `seed terms` rendering, limits, and candidate budgets are
unchanged, and the expansion cannot introduce a term the graph does not
declare. Questions that previously matched only `route`-shaped names can now
seed `Router`, `PathRouter`, and `MethodRouter`.

Discovery also reads preposition phrases as identifier compounds: "to json" and
"from json" add the `tojson`/`fromjson` terms, verified against the bounded
name index like the agent-noun variants. A compound term that equals a declared
name is classified as an exact-name match even in a multi-concept question;
ordinary question words keep their previous alias rank even when a node shares
their spelling. Each concept is also guaranteed one candidate slot before
another concept's postings can fill the pool. The response schema, limits, and
candidate budgets are unchanged.

Discovery also reads the operation verb's direction when the question spells no
preposition: a reading or loading verb names its source `from<Object>`, a
writing or serializing verb names its destination `to<Object>`, and a
conversion or transformation names both, with the object before `into`/`to`
treated as the source and the object after it as the destination. Only
compounds the bounded name index declares are admitted, a preposition the
question already spells is never re-derived, and an infinitive `to` is not a
direction marker, so previously answered phrasings keep their seeds. The
response schema, limits, and candidate budgets are unchanged; the ranking terms
of a question that names an operation and its object can now include the
compound the graph declares (`read json into an object` → `fromjson`).

Immutable history now accepts up to 5 GiB of aggregate authoritative key and
value bytes per realization, raised from 512 MiB. The history schema and
canonical encoding are unchanged, as are the per-key, per-value, per-tree,
JSON-depth, job, and diagnostic bounds. Readers from older Compass releases
continue to reject a realization whose authoritative content exceeds 512 MiB;
deploy a reader containing this limit widening before sharing larger
realizations.

The closed route-stage vocabulary used by `compass.graph/1`,
`compass.query/1`, and `compass.framework-context/1` now includes the additive
`dependency` and `security` values. The query contract manifest and fingerprint
changed with that enum list. CLI and MCP output use the same typed model, and
the bundled viewer validates the same list. Deploy strict readers, manifests,
and generated viewer assets together. Older strict readers must reject these
values; consumers must not coerce either one to `middleware`. The existing
graph and context schema majors are unchanged.

The framework-pack cache identity is now `compass.framework-packs/6`.
Python HTTP routes no longer use the combined `python-web` runtime adapter;
they are owned independently by version-2 `django-python`, version-1
`django-rest-framework-python`, version-2 `fastapi-python`, version-2
`flask-python`, and version-1 `starlette-python` universal packs. Version-1
`pydantic-python`, `sqlalchemy-python`, and `celery-python` own exact model,
persistence, task, queue, canvas, and schedule evidence. The legacy
`enterprise-domain-facts` source pack no longer runs for Python; its non-Python
language list and behavior are unchanged. Strict `compass.framework-context/1`
readers must accept those eight Python IDs and reject the removed combined ID.
Route
and graph schema majors remain unchanged, but cached framework facts must be
rebuilt because pack ownership, evidence provenance, and Flask's default
operation changed. No graph schema major changes: dependency/security stages
and Pydantic schema flow reuse the existing typed stage, node-role, and
`depends_on`, `maps_to`, `produces`, `consumes`, `schedules`, and `triggers`
contracts.

The Django semantics widening retains the existing graph major and additive
edge vocabulary. Exact URL-pattern collections, DRF generated routes, model
relationships/managers, and signal subscriptions can add `routes_to`,
`depends_on`, and `subscribes` records with source anchors. Dynamic patterns,
custom router templates, external inherited viewset methods, and ambiguous
serializer/model targets stay unresolved. Settings, middleware, and admin
registration calls do not publish `registers`: the current pack contract
would require the unrelated bean-container capability, so readers must not
infer those edges from missing output.

Python structural evidence now uses producer version 1. The evidence schema
is unchanged, but previously empty universal callable/type fields can contain
source-proven parameter shapes, literal call types, `type_of`, `returns`, and
call-result bindings. Starred call arguments suppress exact arity; `Any`,
shadowed names, ambiguous return annotations, and unsupported dynamic forms do
not acquire a guessed target. Strict consumers should key caches and audit
baselines by the producer version.

The Grounded Agent Graph feature is additive and opt-in. It does not change
`compass.graph/1`, structural extraction, default query behavior, or immutable
history schemas. Its versioned contracts use the
`compass.agent-graph.*/*` and `compass.agent-knowledge/1` namespaces. Unknown
majors and unknown fields fail closed. `GROUNDED` is a Compass-issued citation
verification state and must not be interpreted as `INFERRED`, `EXTRACTED`, or
proof of semantic truth.

`compass.agent-graph.ingestion-preparation/1` is a read-only additive contract.
It calculates exact Base record and source-evidence digests for a selected Base
Generation; it does not certify, mutate, or publish an assertion. Apply
re-verifies prepared evidence against the pinned Base Generation.

The bundled Compass skill also supports an explicit continuous-enrichment mode
for coding sessions. This is an adapter workflow over the existing versioned
overlay commands, not a new graph or history schema: it keeps a bounded
session-local candidate ledger, publishes only milestone batches, pins each
receipt revision, and requires a complete rebase after a Base Generation
change. Read-only navigation remains the default.

Overlay writes require explicit CLI or server enablement. Existing MCP servers
without Agent Graph configuration advertise no Agent Graph tools. Configured
read-only servers advertise inspection only; HTTP writes additionally require
a distinct write key. Historical composition is selected explicitly and never
changes realization preference or stored history content. No migration of
existing graphs or history is required.

The frontend graph vocabulary is also additive within the pre-release
`compass.graph/1` contract. React-oriented builds may publish the typed
`renders` relationship and the `ui_component`, `hook`, `client_boundary`,
`client_component`, `server_component`, `server_function`, and `data_loader`
node roles. A reader that validates closed edge or role enums must update to a
release containing these values before consuming such a graph; older readers
must fail closed rather than silently dropping them. These values do not make
the corresponding framework pack a completed quality claim: promotion remains
gated by the independent qualification corpus, precision/recall, ambiguity,
limit, and determinism checks in the frontend graph plan.

The checked-in `compass.query/1` enum manifest and fingerprint widen in lockstep
with this vocabulary. Strict query, MCP, CLI, VS Code, and viewer consumers
must use the matching manifest; they must reject an unknown `edgeKind` or
`nodeRole` instead of filtering it into an older response shape.

Markdown semantic table intelligence remains within `compass.graph/1` by using
the established resource-node, qualified-name, source-anchor, and relationship
contracts. Pipe tables continue to publish table, header, row, and cell nodes.
Their semantic labels, stable identities, exact cell-owned references, and
bounded extraction are producer-logic improvements; no new graph wire fields
or schema migration are required. Consumers must continue to reject unknown
graph majors and must not infer document roles from display labels.

Markdown frontmatter intelligence likewise remains within `compass.graph/1`.
Bounded nested YAML metadata publishes through established `config_key` nodes,
Config provenance, exact source anchors, canonical key paths, and `contains`
relationships. Value-independent IDs and JSON Pointer escaping are producer
identity rules, not new wire fields. Generic metadata values are not copied
into graph labels; strict readers need no schema migration.

Swift, Dart, Scala, and Groovy/Gradle now publish through their version-1
universal evidence pipelines. The four pipelines are `Qualified` under the
checked-in universal-evidence promotion decision: they use one bounded,
source-grounded publication route and may change unresolved/ambiguous edges
compared with older direct extraction. Normal cache fingerprints invalidate
affected files; users do not need to delete artifacts manually. Equal names
across Swift/native or JVM-family languages do not by themselves create
cross-language targets.

The same decision records all 14 hard-cut universal pipelines as `Qualified`,
including C#, Go, Java, JavaScript, Kotlin, PHP, Python, Ruby, Rust, and
TypeScript. The status is scoped to each producer's advertised bounded
capabilities and evidence schema/version; changing either requires a fresh
promotion decision and cache regeneration.

Python publishes through version-1 `compass.python` evidence and uses static,
bounded `pyproject.toml` import-root evidence. The corresponding internal
project-evidence identity is `compass.framework-project-evidence/4`. A uniquely
proven `src/` or configured package layout may therefore change qualified names
and stable graph IDs; zero candidates retain the contained repository-relative
identity, while multiple distinct candidates retain that identity plus an
ambiguity diagnostic instead of selecting a root by order. `.py` and `.pyi`
normalize to one module key. Paired source owns graph declarations, stub-only
declarations carry `source_kind: "stub"`, and source/stub disagreement is an
explicit error diagnostic. No Python interpreter, environment discovery,
package installation, or repository import participates in these rules.

A user-visible incompatible change requires:

1. native regression coverage;
2. updated command or format documentation;
3. a migration note;
4. a release note when applicable.

Versioned formats use Compass-owned identifiers. Consumers should reject
unknown major versions instead of attempting legacy fallback behavior.

The architecture viewer is a coordinated hard cut from
`compass.viewer.callflow/1` to `compass.viewer.architecture/1`. Capability
negotiation advertises `architecture_viewer`; consumers must reject an unknown
major. The new payload does not reinterpret old `sections` or `calls` fields:
it publishes typed nodes and relationships once plus source-specific group,
membership, route, omission, and quality projections. The `callflow-json` and
`callflow-html` command names remain available, but direct v1 callflow JSON
consumers must migrate.
Membership records are compact validated indexes into the deterministic node
and per-projection group arrays. Documentation is a first-class All-code source
scope and cannot influence Production architecture.

`compass architecture --format json` may return
`compass.architecture.summary/1` when a declared detail-projection limit is
exceeded. Detailed output is capped at 5,000 nodes and 20,000 relationships.
The summary contains exact graph totals, exact counts for listed kinds,
aggregate counts for omitted kinds, the exceeded limit, and a deterministic
sample of up to 12 largest communities with at most 3 nodes each; ties use
ascending community IDs and node IDs. `kindCountPolicy` orders each kind map by
ascending name and caps it at 64 safe names; counts for omitted kinds are
aggregated exactly as `otherNodes` and `otherRelationships`. Sampled IDs are at
most 1,024 bytes. Sample text fields
are bounded to 512 characters, escape control and bidi characters, and list
changed fields in `boundedFields`; unrepresentable sampled IDs are counted in
`omittedSampleNodes`. It sets `detailsOmitted` and does not reuse the viewer
schema for incomplete data. Agent JSON reports
`compass.architecture.summary-agent-view/1` and includes the underlying schema
as `summarySchema`.

Before the first compatibility-stable release, Compass hard-resets active
internal extraction, cache, publication, store-index, query-index/ranker,
overview, qualification, and semantic-diff identities to v1. Provisional
higher-numbered artifacts are unsupported and are not migrated or interpreted
alongside earlier v1 prototypes. Discard existing pre-release artifacts and
rebuild project output with the current binary; disposable query indexes rebuild
automatically.

The release workflow publishes `compass-release.json` with schema
`compass.release/1`. `compass upgrade` retrieves that bounded static manifest
through the GitHub release-download path, requires one exact artifact for each
running binary's target, and rejects unknown schemas, unstable or mismatched
versions/tags, duplicate or invalid targets, invalid sizes, and invalid SHA-256
digests. Additional bounded target entries remain forward-compatible. Archive
downloads use the immutable validated tag rather than a mutable latest URL.

Current output uses visible Compass-owned paths: `snapshots/`,
`current-snapshot`, `root-artifacts-complete`, and
`store/`. Snapshot-local state likewise uses concise names such as
`build-state.json` and `analysis.json`. This is an unconditional hard cut:
the runtime has no hidden-layout detector, compatibility reader, path mapping,
or in-place migrator. Output created by an older layout must be archived or
removed before rebuilding. Repository configuration under `.compass/` remains
unchanged because it is not output state.

Versioned history remains on realization schema 1, store-format root
`compass/store-format/v1`, and the `compass/v1` realization-root namespace.
The visible output-path cutover does not change those serialized contracts.
Historical realizations containing former hidden artifact paths are not mapped
or rewritten; run `compass history rebuild REV` for a revision that must use
the current visible artifact layout.

The current local build publishes `graph.json` (`compass.graph/1`) directly
under the selected output root by default. It also materializes
`GRAPH_REPORT.md`, `manifest.json`, and optional `graph.html` at that stable
root path. Compass retains an immutable snapshot behind those conventional
paths as its coherent internal authority.

The additive `compass.graph/1` endpoint matrix accepts exact `calls` edges to
`property` nodes. This represents source-proven callable fields, callbacks,
and object properties without changing node or edge identity; consumers that
validate endpoint kinds should accept this existing-major widening.

The frontend graph vocabulary is also additive within the pre-release
`compass.graph/1` contract. React-oriented builds may publish the typed
`renders` relationship and the `ui_component`, `hook`, `client_boundary`,
`client_component`, `server_component`, `server_function`, and `data_loader`
node roles. A reader that validates closed edge or role enums must update to a
release containing these values before consuming such a graph; older readers
must fail closed rather than silently dropping them. These values do not make
the corresponding framework pack a completed quality claim: promotion remains
gated by the independent qualification corpus, precision/recall, ambiguity,
limit, and determinism checks in the frontend graph plan.

The checked-in `compass.query/1` enum manifest and fingerprint widen in lockstep
with this vocabulary. Strict query, MCP, CLI, VS Code, and viewer consumers
must use the matching manifest; they must reject an unknown `edgeKind` or
`nodeRole` instead of filtering it into an older response shape.

Large universal-evidence collections now degrade explicitly instead of
silently publishing file scaffolding. Compass retains source declarations and
safe exact relationships in deterministic bounded partitions, records omitted
relationship candidates in publication statistics, and adds the
`universal_resolution_partial` error diagnostic to `compass.graph/1`. A build
that emits this diagnostic publishes the useful partial artifact but exits
nonzero. This is an additive diagnostic and completeness behavior change; the
graph schema major and identities of successfully resolved records are
unchanged.

The self-contained HTML viewer embeds `compass.viewer.workbench/1`, an additive
ordered container for code, call, impact, affected, architecture, historical,
and artifact-lens models. Each view carries explicit bounded coverage. Plain
`compass export json` remains `compass.viewer.graph/1`; requesting one or more
views, or using `compass export workbench-json`, returns the workbench contract.
Consumers must reject an unknown workbench major version. The HTML DOM and CSS
remain presentation details rather than machine contracts.
Standalone HTML may additionally embed optional, presentation-only source
navigation metadata for a recognized Git forge and full source commit. This
metadata is outside `compass.viewer.workbench/1`; `workbench-json` and the
versioned graph/view contracts are unchanged.
Rich document nodes may carry an optional additive `document` object in
`compass.viewer.graph/1`. It contains bounded native/OCR provenance, typed
page/slide/sheet locators, confidence, and OCR geometry for the viewer; older
readers may ignore the field, while strict readers should preserve unknown
nested fields and fail closed only on an unknown contract major.
When present, `document.previews` lets viewers display the normalized Office
snapshot selected by an OCR candidate and map its source polygon into the
embedded-image region. Missing or omitted preview images remain explicit
diagnostics and never erase native/OCR evidence.
Structural builds publish a validated `store.sqlite3` sidecar and typed
`store.ref` selector by default; `--store json` explicitly opts out. Typed code
queries prefer that validated sidecar by default, while `--engine json`
explicitly selects the permanent JSON engine. Once a store reference is
present, corruption or a selector mismatch fails closed instead of silently
querying a different realization. The SQLite file and reference are internal
realizations of the backend-neutral `compass-store` contract, not a stable SQL
schema or pointer format that consumers may query directly.

The additive `compass ask` command continues to route bounded questions to the
typed `compass.query/1` operations. Its agent projections now retain the parsed
symbol/source/target operands used by that operation, with the original question
in `request.question`. This corrects headlines, evidence basis, and follow-up
actions that previously treated the whole question as a symbol. Agent-answer
subject lookup shares the query engine's existing name normalization (case,
leading dots, and trailing empty parentheses), retaining exact node-ID lookup
and requiring a unique normalized name. Existing schema
majors and raw query responses are unchanged. Text cursors whose primary ordering
changed are rejected by the existing prefix check; reissue the question.
Ambiguous typed answers now use the operation as their answer basis and ask
for exact node IDs, with IDs printed for every retained ambiguity candidate.
They no longer describe the first candidate as a fallback answer or imply
that an ambiguous path request proved disconnection. Schema majors and raw
query responses are unchanged. Existing text cursor prefix checks reject
pages whose candidate rendering changed; restart that query from page one.
Plain `compass query` against a typed graph
now defaults to `compass.query.discovery/1`; `--dfs` and `--context` compose
with discovery. Explicit `--traverse` or legacy-only `--budget`/`--page`
preserve the established text traversal and reject discovery controls.
CompassQL and explicit typed query commands remain unchanged. Discovery text
pagination now uses the versioned `compass.query.discovery-text-page/2` cursor.
Text is concise by default, `--evidence` restores full provenance detail, and
the selected tier is bound into the cursor. Version-1 cursors fail explicitly
rather than resuming into a different representation. The default text-page
budget is 8,000 approximate tokens. JSON rejects those presentation-only
controls and the discovery JSON schema remains `compass.query.discovery/1`.

Exact-looking discovery or typed-query operands that have only fuzzy or lexical
candidates now carry a structured `no_match` diagnostic before any fallback
content or bounded natural-query execution. Dedicated `compass path` endpoints
require unique exact identities;
weighted path selection prefers structural evidence over reference/document
shortcuts and reports an eligible shorter-but-weaker alternative separately.
These are human-query semantic changes, not graph or JSON schema changes.
`compass ask --at REV` uses the same immutable trusted `compass.graph/1`
realization selection as revision discovery. The response remains the unchanged
`compass.query/1` contract; an older realization without that trusted graph is
rejected and must be rebuilt.

Default discovery JSON remains the strict `compass.query.discovery/1` shape.
The focused default neighborhood is 64 nodes and 128 edges. The existing hard
ceilings remain 500 nodes and 1,000 edges, and callers that require the wider
neighborhood can continue to request it explicitly with `--max-nodes 500
--max-edges 1000` or the equivalent typed request fields. This changes only
default breadth; the v1 request and response schemas, ordering, truncation,
and omission contracts are unchanged.
The additive `--result-envelope` option requires `--format json` and returns a
typed `compass.query.discovery-result/1` envelope containing the unchanged v1
result plus its query-owned `semanticResultDigest`. The digest is computed from
canonical v1 semantic response bytes; the digest field is outside that result,
so the v1 payload and its byte/shape contract remain unchanged.

Clustered updates publish `orientation.json` (`compass.orientation/2`) from the
same fitted model as `GRAPH_REPORT.md` and include it in the coherent snapshot
and build state. The additive `blindSpots` field carries the versioned,
bounded graph-insights report with witnesses, exact omission counts, and
limits. `compass export orientation-json` and
`compass://orientation` validate that its generation, source/configuration
identity, commit, graph summary, and exact streamed `graph.json` artifact
digest match the selected guarded graph. A direct or historical graph without
that coherent artifact fails explicitly.

MCP structured tool results use the `compass.mcp.tool-result/1` envelope. Its
`result` retains the domain schema and domain truncation fields unchanged;
`transportTruncation` separately reports the MCP byte bound. A response that
would exceed that bound fails with typed required/limit/omitted byte metadata
instead of publishing a partial semantic result.
Natural discovery results additionally expose the same query-owned
`semanticResultDigest` in this transport envelope, enabling direct/persistent
result parity checks without requiring an agent client to invent a digest.
Task-oriented results use strict `compass.task-context/2` and
`compass.task-context-profile/1` contracts through `compass context` and MCP
`task_context`. Exact identity resolution, digest-verified source, provenance,
omissions, and domain truncation remain inside the result; fuzzy candidates
are never selected. The domain digest excludes only its own field and the
observational response-byte count.

Structural operands use the same bounded exact, alias, term, and typo recall
channels as search. A unique relationship-role seed may disambiguate a
non-exact operand, while duplicate exact names remain an explicit
`ambiguous_match`. Node-trail operations are directed from the supplied source
to target. A route that exists only when ignoring edge direction returns `direction_mismatch`
instead of publishing a misleading path; callers that need the reverse route
must swap the operands. This adds one typed diagnostic variant to
`compass.query/1`.

The Rust library's `query_natural_profiled` API returns a separate
`compass.query-execution-profile/1` envelope. It does not add timing or work
fields to `compass.query/1`, so ordinary responses remain deterministic and
backend-neutral.

Typed symbol search now unconditionally uses `query-ranker/1`. The internal
`COMPASS_QUERY_RANKER_PROFILE` experiment switch and v1 runtime fallback have
been removed. This does not change the `compass.query/1` schema, but intentional
score and ordering improvements can change which equally lexical candidate is
ranked first; ordering remains deterministic and backend-neutral.

Discovery term indexes preserve their existing full tokens and add bounded
camel-case, acronym, and underscore subwords derived from raw symbol names,
qualified names, and aliases. They also add exact relationship-term postings
from source-backed callable nodes through direct `calls` edges whose evidence
is entirely exact and non-heuristic. Relationship postings use only the called
target's terminal symbol name; namespace and owner terms from its qualified
name remain available to direct lexical recall but do not become caller
evidence. Parallel edges are deduplicated for this recall index; inferred,
ambiguous, mixed-confidence, heuristic, source-less, and non-callable sources
do not participate.

Direct symbols and candidates with at least two trusted relationship concepts
share one deterministic behavior-ranking channel. They are ordered by
production status, bounded operation-predicate alignment, direct
terminal/owner concept coverage, semantic kind, field and predicate precision,
relationship concept coverage, distinct supporting targets, and evidence
confidence. A relationship candidate keeps its lexical or alias source when
it also has direct indexed evidence; only relationship-only recall is labeled
as a relation seed. Fixed whole-token operation families (including
persistence, dispatch, invocation, processing, recognition, refresh,
resolution, and scheduling) affect ranking only: they cannot add a posting,
candidate, relationship concept, or relation eligibility. Equal evidence
vectors remain explicitly ambiguous.
Lexical natural-query alternatives require the same channel, operation,
relationship, and calibrated score rank before they are labeled ambiguous.
This removes false ambiguity between a specifically ranked operation or
representation and a weaker same-name/helper candidate. Equal-rank candidates
and duplicate exact-name lookups remain explicit ambiguity. Exact matches to
the same source-backed declaration name remain ambiguous across differences
in kind, signature, owner, and ranking evidence; ranking does not prove which
declaration the user meant.

Within prose, underscore spellings and mixed-case internal capitals receive
bounded literal-name lookup before behavior recall. Only declared-name matches
receive exact-name priority. This changes candidate ordering without changing
the discovery schema or graph format. Truncated name postings or candidate
admission cannot prove uniqueness, and generic operation ranking cannot
override that uncertainty. Single-word capitalized names continue through the
existing ranking unless the whole question is an exact name.

For explicit action predicates, discovery first reads one compact exact-term
index restricted to source-backed operation-role declarations. It may finish
from that index only when the complete role set proves that the top role
matches the explicit predicate, covers the query subject, and dominates
omitted non-role types; location-style questions otherwise continue through
general recall. A subject-only `Builder` match cannot suppress a more specific
method. A second compact channel projects the existing full
term postings onto source-backed type declarations. Discovery may finish from
that channel only when it is complete, contains every requested seed slot, and
the existing ranker proves each selected declaration covers the query subject
and dominates every omitted non-type. This can intentionally keep a direct representation type ahead of a
less-specific operation-role type that max-level inferred relationship evidence
would otherwise promote. Legacy snapshots use at most 18 deterministic bounded
role-name/intersection probes and fall through to general recall when the
declaration capability is absent. Discovery then performs at most eight general
multi-concept term-index intersections before independent term unions. Every
read spends the same candidate, posting, object, byte, and probe budgets;
exhaustion remains explicit truncation rather than an empty result. A complete
exact-name lookup can prove its top channel despite truncation in lower recall
channels, while duplicate exact names remain ambiguous.

For a question with at least three distinct concepts, discovery now requires
one exact identifier/name, a source-backed operation or representation type,
at least two direct matched concepts, or trusted multi-concept relationship
evidence somewhere in the ranked pool. A composite identifier containing at
least three concepts requires an exact name or ID. If recall finds only
isolated generic subword hits, the response is an explicit `no_match` instead
of presenting unrelated symbols as an answer. This tightens result admission
without changing the `compass.query.discovery/1` schema or deterministic rank
ordering of admitted candidates.
An explicit `path from <symbol> to <symbol>` question is admitted when recall
proves two distinct exact terminal symbol references. This narrow structural
case preserves path discovery without admitting generic multi-concept noise.

Discovery traversal bounds adjacency reads by remaining node capacity and
stops endpoint hydration at the node cap. Store-backed final edge assembly
scans unit-valued outgoing references, rejects targets outside the selected
subgraph before record hydration, and resolves the remaining edge IDs through
a bounded shared tree traversal. This preserves canonical parallel-edge order
and exact edge omissions when the reference scan completes; a shared expansion
limit still produces explicit incomplete counts. Multi-concept exact-term
recall intersects compact node IDs before hydrating the surviving node records.
Exact term candidates and adjacency records use bounded multi-key tree walks so
immutable branch and leaf
objects are decoded once per batch. A pinned request reader retains only
digest-verified, decoded, schema-validated tree objects in an 8 MiB envelope
with a 7 MiB decoded-object budget and a 1,024-object ceiling. Branches are
retained preferentially and leaves use LRU eviction; cache hits do not bypass
any logical item, byte, object, depth, or truncation accounting.

The immutable store records identifier, operation-role, declaration, and
relationship capabilities as separate empty reserved postings in its existing
additive terms root, which older same-major readers ignore. Snapshots without
the operation-role capability remain readable and use the bounded role
fallback. Snapshots without the declaration capability remain readable and
continue through general recall; no candidate meaning is invented from either
missing accelerator. Relationship membership is also stored as a bounded
unit-valued `(source, term)` key so a complete sparse posting can prove
membership in one truncated dense posting without scanning adjacency. The v2
relationship capability also stores bounded unit-valued
`(source, term, target)` evidence so ranking can count distinct query-supporting
callees without inflating parallel calls or one callee that matches multiple
concepts. Current readers still open snapshots without either capability but
report incomplete discovery coverage; rebuild the graph to make discovery
recall equivalent across the JSON and store engines. The disposable SQLite
query cache adds `relationship_terms(term, source_id)` and
`relationship_term_targets(term, source_id, target_id)` tables, uses internal
format v7, and is rebuilt automatically.

Optional MCP query feedback remains local and disabled by default.
`COMPASS_QUERY_LOG=<path>` writes the versioned `compass.query-log/1` JSONL
contract up to a 16 MiB file bound. The review importer accepts only its
bounded `question` field and emits a separate
`compass.query-review-candidates/1` queue; neither format is a judgment corpus.

Structural `init`, `update`, `extract`, and `watch` builds publish
`program.json` only when `--program` or `--program-artifact` is selected. The
legacy `--no-program` flag remains accepted and continues to request the
structural-only profile. Program inspection commands remain read-only and
require an existing canonical Program IR artifact.

The optional `compass.scip-manifest/1` companion now accepts an additive
`managed_analyzer` member with the strict
`compass.managed-analyzer-profile/1` contract. Existing companions without the
member retain their prior generic SCIP behavior. Managed Python provider IDs
include the frozen profile and artifact digests, so Python environment, stub,
project-configuration, permission, limit, or producer changes invalidate the
artifact cache deterministically. Unknown profile majors and non-complete or
stale profiles fail closed. No analyzer runtime, installation, network access,
or project execution is added to structural builds.

Structural build commands accept the additive
`--inference-level low|medium|high|max` profile input. `low` is the default and
publishes exact relationships only. `medium`, `high`, and `max` remain explicit
opt-ins; `max` preserves the former complete-inference behavior. The selected
level is part of the build profile and configuration digest, so the default
cutover republishes a coherent graph without changing `compass.graph/1`.
Schema-1 build profiles that omit the field still deserialize as historical
`max`; new default-low profiles serialize `"inference_level":"low"`
explicitly. There is no environment switch or automatic breadth fallback.

## Pull-request intelligence contract

`compass review` and MCP `review_pull_request` add the strict
`compass.pr_intelligence.report/1` machine contract. Unknown fields, unknown
enum values, malformed digests, invalid references, and unknown major versions
fail explicitly. Finding identities use `cmpprv1:<sha256>`; the advisory
integer rubric is version 1; each deterministic gate has its own rule version.
Presentation formats and the reusable GitHub Action consume this report and do
not redefine its semantics.

When either side of a review has a preferred realization or repository history
profile with noncurrent engine fields, Compass replaces every engine-owned
field with the running contract and then validates the complete reconstructed
profile before materializing both revisions. The persisted `compass_version`
is provenance, not a compatibility gate: review does not parse, order, or
allowlist release numbers. This also applies when both revisions already have
preferred realizations. Reconciliation proceeds only when their user-selected
options are identical after reconstruction. Historical realizations remain
immutable and queryable. Malformed or unsupported profile shapes and different
user options fail explicitly. When the supported profile shape changes, Compass
uses a hard cutover rather than accumulating release-specific migrations.

Dependency findings in `compass.semantic_diff.report/1` may now carry the
optional strict `dependency_topology` object. It records source/target community
IDs when present and bounded directed-cycle participation when the snapshot
adapter can prove it. Semantic-diff derived-cache engine version 2 prevents
older cached reports from masquerading as current topology evidence.

The PR Intelligence report binds full Git revision IDs, graph/profile identity,
and an evidence manifest. A profile mismatch is an error. Conflicts and
incomplete evidence remain explicit and cannot become a clean gate result.
Advisory risk is never a merge gate. The Action supports only
`fail-on: none|deterministic`, where `deterministic` consults typed
`GateResult::Fail` states rather than risk band, score, SARIF level, or prose.
Its required `compass-version` input must name an exact released version
containing `compass review`; there is no fallback binary version.
On a fresh checkout, the Action explicitly materializes the target revision
with the local `--code-only` history profile before invoking review; it does
not silently downgrade a configured semantic profile.

PR review finding statements now resolve retained entity identities to
human-readable names. Stable entity identities remain in the canonical finding
`source_entities` and `target_entities` fields, so this presentation change
does not alter finding fingerprints or machine traceability.

Human-facing review projections use short revision and fingerprint references,
plain-language status labels, and relationship-only witness summaries. Exact
revision IDs, graph entity IDs, fingerprints, and witness endpoints remain in
canonical JSON and SARIF. Text and Markdown are presentation formats and must
not be parsed as machine contracts.

This is additive in the `0.3.x` line. Existing `compass prs`, graph, history,
and MCP contracts are unchanged; `compass diff` gains only the optional typed
topology field above. Consumers that adopt the new
report must reject unknown majors and validate `report_digest`. See the
[PR Intelligence reference](docs/reference/pr-intelligence.md).

`compass review --readiness` and MCP `pr_readiness` add the strict
`compass.pr-readiness/1` envelope. It references the unchanged canonical report
digest and exact revisions/profile/evidence identity. Documentation drift is
advisory-only, both extraction fingerprints remain explicit, unavailable test
evidence remains unknown, and bounded local ownership failure is an explicit
omission. This addition does not change
`compass.pr_intelligence.report/1`, its digest, or existing review projections.

The `extract --code-only` profile excludes document extractors from structural
node and edge publication while retaining the scanned file inventory and its
status records.

## Community detection profile cutover

Typed clustered graphs use the complete profile
`seeded-leiden-modularity/v1` + `typed-evidence-undirected/v1` +
`community-quality/v1` + `fixed-resolution/v1`, seed `42`, and
`community-limits/v1`. The default resolution is fixed at `1`; an explicit
`--resolution N` remains a single fixed positive finite resolution. The
bounded three-candidate selector has identity `bounded-multiresolution/v1` but
remains qualification-only until its complete pinned-corpus release matrix
passes the latency, memory, stability, and quality gates.

This is a compatibility-sensitive membership cutover without a
`compass.graph/1` schema change. Community numeric IDs, membership, labels,
reports, and architecture groupings may change. Base Graph node and edge
identity, direction, multiplicity, anchors, provenance, and canonical encoding
do not change as a consequence of clustering. The complete profile enters the
configuration digest and current/history build profiles, so old output is
rebuilt coherently rather than partially reused.

Clustered typed builds add strict `compass.community-quality/1` at
`community-quality.json`. Readers must validate its self-digest, graph
generation, exact canonical graph digest, and profile identity and reject
unknown majors or fields. Missing evidence on an older, schema-less legacy, or
unclustered graph means unavailable. Direct reclustering of a schema-less
legacy graph retains `seeded-louvain/v1` compatibility and publishes no quality
sidecar.

Historical realizations and their sidecars are immutable. Compass never
substitutes Louvain results under a Leiden profile or interprets one profile's
member IDs as another profile's result. Existing `cohesion` remains the public
density projection, now calculated by the shared quality evaluator. That
projection is the share of a community's possible member pairs that carry an
internal edge: a self-loop is an internal edge and stays in the published edge
and weight inventories, but it is not a member pair, so it never raises the
share or the hierarchy cohesion that mirrors it above one.

Clustered typed builds additionally publish strict
`compass.community-hierarchy/1` at `community-hierarchy.json`. The artifact is
additive: nothing in `graph.json`, `graph-overview.json`, `orientation.json`,
CompassQL, MCP results, or query semantics reads it, so a consumer that ignores
it behaves exactly as before. Readers must validate its self-digest, graph
generation, canonical graph digest, and profile identity and reject unknown
majors or fields. Absence means level navigation is unavailable — an older
graph, a schema-less legacy recluster, or a `--no-cluster` build — not an empty
hierarchy. Levels describe the same partition the quality artifact describes;
`finestSignature` binds the two. `compass export hierarchy-json` emits the
artifact unchanged and fails for an unknown major or a mismatched graph.

The levels themselves are data, not schema: a build publishes the levels the
evidence supports and records the achieved counts, so no consumer may assume a
level count or a group count. Two rules older artifacts can violate and new
builds cannot: a level never holds one group (the whole repository as a single
node) and the location cut may publish a bounded overshoot of its target —
recorded as `mergeEvidence.escapedSingleBucket` — instead of one bucket holding
every named group. Readers that ignore `mergeEvidence` behave exactly as
before, and exports still open on a level an older artifact published.

Group ids inside that artifact are durable: `h<level>-<signature16>` over the
group's member signatures. Reconciliation rewrites an id to the previous
build's when a group survives, so consumers may key on `id` across rebuilds, but
must treat a changed `signature` as changed membership and must handle
`ambiguous` events rather than assuming every previous group has exactly one
successor. The identity ledger `community-hierarchy.json.sig` is additive too:
absence means identity can only be derived from the artifact's own signatures.
Any change to the signature algorithm or the reconciliation thresholds is a
compatibility-sensitive change and needs a version bump.

`compass.community-hierarchy-diff/1` is the comparison surface for two
generations. It is additive to the history workbench view
(`compass.viewer.workbench/1`, `kind: "history"`), appears only when both
realizations published a hierarchy, and must be rejected by consumers when its
schema major is unknown. Absence means the comparison is unavailable, not that
nothing changed; a present diff with zero change counts and no events is the
"nothing structural changed" case.

## Compass Store release contract

The first supported local store line is `0.3.x`. Its logical machine formats
are versioned independently:

| Contract | Major | Support in `0.3.x` |
| --- | --- | --- |
| Graph JSON | `compass.graph/1` | Permanent compatible engine; direct input, publication, inspection, interchange, recovery, and deterministic export |
| Common key-value API | `compass.store/1` | Supported by the local SQLite adapter and the library-only redb adapter |
| Immutable graph snapshot | `compass.store.graph-snapshot/1` | Same-major reopen and validation |
| Store reference | `compass.store.ref/1` | Required to bind the selected snapshot to `graph.json` |
| Backup bundle | `compass.store.backup/1` | Validated by `compass store restore` into a new directory |

Patch releases may reopen a matching major. Unknown majors, pre-release
physical files, mismatched adapters, and invalid references fail explicitly and
must be rebuilt; they are never silently treated as empty data. The SQLite
tables/WAL, redb file, object-key spellings, and query-index caches remain
rebuildable implementation details. See the [store operations guide](docs/guides/compass-store-operations.md)
and [migration notes](MIGRATION.md).

The CLI currently selects SQLite for a validated local sidecar.
`compass-store-redb` is a separate library adapter used by conformance and
qualification tests; it is not a CLI or packaging dependency. PostgreSQL and
DynamoDB are future adapters, not supported release backends. No local store
command accepts cloud credentials, endpoints, or TLS configuration.

The default published locations are `DIR/graph.json` and the validated SQLite
sidecar under the selected `--out DIR` (default `compass-out/`). The build
publishes `store.ref` beside the current snapshot's `graph.json` and keeps the
shared database at `DIR/store/store.sqlite3`; `--store json` omits those
sidecars while retaining the same JSON artifact.
`compass store status|validate|backup|restore` are the supported operational
surface. Backups are digest-bound directories and restores never overwrite an
existing destination. Local publication retains two complete snapshots and
performs bounded reachability GC; remote leases, service quotas, and
distributed GC remain deferred. The local API enforces bounded values, scans,
transactions, and request work. Current `compass.store.graph-index/1`
snapshots do not impose an aggregate canonical-payload or record-count limit:
their manifest uses `u64` byte and record counts, while each immutable tree
object, write batch, scan, and query remains independently bounded. The legacy
monolithic `compass.store.graph-snapshot/1` compatibility API still enforces
its 2 GiB materialized-payload limit. `compass store status|validate|backup|restore`
stream graph and database digests through fixed-size buffers and traverse the
reachable immutable tree objects with bounded cache and path memory; they do
not depend on the whole-JSON reader limit.

Directional adjacency indexes advertise an edge-ID-order capability. Bounded
JSON and store queries select the same canonical edge-ID prefix across the
requested relationship kinds before applying heuristic filtering. A sidecar
without that capability remains available for validation, backup, JSON export,
and recovery, but directional store queries reject it with a rebuild
instruction; they never reinterpret endpoint-ordered keys as edge-ordered
results.

The hard-cut boundary is the sidecar and all disposable indexes. When a
physical format is invalid or outside the support window, preserve
`graph.json`, run `scripts/rebuild_compass_store.sh`, or explicitly select
`--engine json`. The JSON engine does not require a database and is not a
migration fallback scheduled for removal.

Markdown graph extraction is a structural, extensible projection. New
document/block attributes and bounded diagnostic extensions may appear without
changing node identity; consumers must preserve unknown attributes, edge
direction, multiplicity, and source ranges. Markdown frontmatter is part of
the file hash, so metadata-only edits invalidate compatible extraction/cache
entries and are rebuilt under the current extraction semantics version.
Structural blocks now carry section-qualified names and automatic duplicate
heading slugs use deterministic source-order suffixes. Project resolution may
connect local document links to a unique heading, document root, directory
index, or source-file inventory node. Extension inference and wikilink stem
matching are bounded closed rules; ambiguity or a missing fragment never picks
one candidate or falls back to the document root. These additive attributes and
relationships rebuild under extraction semantics v12; graph schema v1 and
existing relationship direction remain unchanged.

HTML (`.html`/`.htm`) now has the same source-driven structural contract. HTML
nodes and link evidence preserve exact source ranges and deterministic order;
`script`, `style`, `template`, and `noscript` subtrees are excluded. The new
`html_*` metadata and diagnostic extensions are forward-compatible attributes.
The extraction semantics version is bumped so older realizations are not
silently reused. URL ingestion uses the same parser-backed HTML normalizer and
does not fetch discovered links.
If semantic enrichment is enabled, these structural nodes and relationships
remain in the published graph; provider concepts are additive and cannot
replace the local realization.

## Attribution

Compass was inspired by
[Graphify](https://github.com/Graphify-Labs/graphify). This attribution records
project lineage only; it does not create a runtime, testing, or compatibility
dependency between the products.
