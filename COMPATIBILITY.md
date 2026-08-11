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

## Evolving contracts

A user-visible incompatible change requires:

1. native regression coverage;
2. updated command or format documentation;
3. a migration note;
4. a release note when applicable.

Versioned formats use Compass-owned identifiers. Consumers should reject
unknown major versions instead of attempting legacy fallback behavior.

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
or rewritten; rebuild those revisions when they must be materialized with the
current visible artifact layout.

The current local build publishes `graph.json` (`compass.graph/1`) directly
under the selected output root by default. It also materializes
`GRAPH_REPORT.md`, `manifest.json`, and optional `graph.html` at that stable
root path. Compass retains an immutable snapshot behind those conventional
paths as its coherent internal authority.
Passing `--store sqlite` also publishes a validated `store.sqlite3`
sidecar and typed `store.ref` selector. Typed code queries use JSON by default;
`--engine store` explicitly selects and validates the sidecar. The SQLite file
and reference are internal realizations of the backend-neutral `compass-store`
contract, not a stable SQL schema or pointer format that consumers may query
directly.

The additive `compass ask` command continues to route bounded questions to the
typed `compass.query/1` operations. Plain `compass query` against a typed graph
now defaults to `compass.query.discovery/1`; `--dfs` and `--context` compose
with discovery. Explicit `--traverse` or legacy-only `--budget`/`--page`
preserve the established text traversal and reject discovery controls.
CompassQL and explicit typed query commands remain unchanged. Discovery text
pagination uses the versioned `compass.query.discovery-text-page/1` cursor;
JSON rejects those presentation-only controls.

Default discovery JSON remains the strict `compass.query.discovery/1` shape.
The additive `--result-envelope` option requires `--format json` and returns a
typed `compass.query.discovery-result/1` envelope containing the unchanged v1
result plus its query-owned `semanticResultDigest`. The digest is computed from
canonical v1 semantic response bytes; the digest field is outside that result,
so the v1 payload and its byte/shape contract remain unchanged.

Clustered updates publish `orientation.json` (`compass.orientation/1`) from the
same fitted model as `GRAPH_REPORT.md` and include it in the coherent snapshot
and build state. `compass export orientation-json` and
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

Typed symbol search now unconditionally uses `query-ranker/2`. The internal
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

Discovery performs at most eight deterministic multi-concept term-index
intersections before independent term unions. Intersection reads spend the
same candidate, posting, object, byte, and probe budgets as all other recall;
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

Discovery traversal bounds adjacency reads by remaining node capacity and
stops endpoint hydration at the node cap. Store-backed final edge assembly
scans unit-valued outgoing references, rejects targets outside the selected
subgraph before record hydration, and resolves the remaining edge IDs through
a bounded shared tree traversal. This preserves canonical parallel-edge order
and exact edge omissions when the reference scan completes; a shared expansion
limit still produces explicit incomplete counts. Exact term candidates and
adjacency records use bounded multi-key tree walks so immutable branch and leaf
objects are decoded once per batch. A pinned request reader retains only
digest-verified, decoded, schema-validated tree objects in an 8 MiB envelope
with a 7 MiB decoded-object budget and a 1,024-object ceiling. Branches are
retained preferentially and leaves use LRU eviction; cache hits do not bypass
any logical item, byte, object, depth, or truncation accounting.

The immutable store records identifier and relationship capabilities as
separate empty reserved postings in its existing additive terms root, which
older same-major readers ignore. Relationship membership is also stored as a
bounded unit-valued `(source, term)` key so a complete sparse posting can prove
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

Structural build commands accept the additive
`--inference-level low|medium|high|max` profile input. `max` remains the
default and preserves complete publication behavior. Non-default levels are
part of the build profile and configuration digest; changing the level
republishes a coherent graph without changing `compass.graph/1`.

## Pull-request intelligence contract

`compass review` and MCP `review_pull_request` add the strict
`compass.pr_intelligence.report/1` machine contract. Unknown fields, unknown
enum values, malformed digests, invalid references, and unknown major versions
fail explicitly. Finding identities use `cmpprv1:<sha256>`; the advisory
integer rubric is version 1; each deterministic gate has its own rule version.
Presentation formats and the reusable GitHub Action consume this report and do
not redefine its semantics.

The report binds full Git revision IDs, graph/profile identity, and an evidence
manifest. A profile mismatch is an error. Conflicts and incomplete evidence
remain explicit and cannot become a clean gate result. Advisory risk is never a
merge gate. The Action supports only `fail-on: none|deterministic`, where
`deterministic` consults typed `GateResult::Fail` states rather than risk band,
score, SARIF level, or prose.

This is additive in the `0.3.x` line. Existing `compass diff`, `compass prs`,
graph, history, and MCP contracts are unchanged. Consumers that adopt the new
report must reject unknown majors and validate `report_digest`. See the
[PR Intelligence reference](docs/reference/pr-intelligence.md).

The `extract --code-only` profile excludes document extractors from structural
node and edge publication while retaining the scanned file inventory and its
status records.

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

The default published location is `DIR/graph.json` under the selected
`--out DIR` (default `compass-out/`). A `--store sqlite` build additionally
publishes `store.ref` beside the current snapshot's `graph.json` and keeps the
shared database at `DIR/store/store.sqlite3`.
`compass store status|validate|backup|restore` are the supported operational
surface. Backups are digest-bound directories and restores never overwrite an
existing destination. Local publication retains two complete snapshots and
performs bounded reachability GC; remote leases, service quotas, and
distributed GC remain deferred. The local API enforces bounded values, scans,
transactions, graph sizes, and request work.

The hard-cut boundary is the sidecar and all disposable indexes. When a
physical format is invalid or outside the support window, preserve
`graph.json`, run `scripts/rebuild_compass_store.sh`, or continue with the
default JSON engine. The JSON engine does not require a database and is not a
migration fallback scheduled for removal.

Markdown graph extraction is a structural, extensible projection. New
document/block attributes and bounded diagnostic extensions may appear without
changing node identity; consumers must preserve unknown attributes, edge
direction, multiplicity, and source ranges. Markdown frontmatter is part of
the file hash, so metadata-only edits invalidate compatible extraction/cache
entries and are rebuilt under the current extraction semantics version.

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
