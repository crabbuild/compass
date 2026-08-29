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

## Agent Skill compatibility

The portable `compass` Agent Skill remains the canonical compatibility entry
point and is installed unchanged. Compass also installs six additive sibling
skills: `compass-navigate`, `compass-debug`, `compass-change-impact`,
`compass-architecture`, `compass-index-maintenance`, and `compass-mcp-setup`.
Their lower-kebab directory names match their `SKILL.md` names.

Each skill tree has an independent checksum ownership manifest. Equivalent
reinstall is idempotent, shared platform consumers are retained until the last
consumer uninstalls, and unowned or modified trees are preserved. Older clients
that activate only the `compass` umbrella remain compatible and require no
migration.

The additive `compass agent` namespace does not replace the installer.
`compass agent install` delegates its remaining argument vector unchanged to
`compass install`; equivalent invocations retain byte-identical output, exit
status, and managed files. Inventory, doctor, bundle, and validation machine
outputs use explicit `compass.agent-list/1`, `compass.agent-doctor/1`,
`compass.agent-bundle/1`, and `compass.agent-validation/1` identifiers.
Unknown major versions must be rejected. Portable exports contain only the
current seven-skill collection and native MCP configuration; cross-harness
plugin packages are a separate contract.

## MCP transport compatibility

Both stdio and Streamable HTTP require MCP 2026-07-28. Older protocol revisions
are rejected rather than negotiated. Current clients begin with
`server/discover`; subsequent requests carry the protocol's per-request
metadata. HTTP requests additionally carry `Mcp-Protocol-Version`,
`Mcp-Method`, and applicable parameter headers, and Compass neither issues nor
requires `Mcp-Session-Id`.

Compass does not ship a legacy MCP-2025 transport mode. `--stateless` remains an
accepted compatibility spelling for the HTTP default. `--session-timeout`
remains accepted in 0.4.x, validates its existing numeric grammar, emits a
deprecation warning, and is ignored because no HTTP session exists. It is
scheduled for removal in Compass 0.5.0. The warning does not change success,
usage-error, or runtime-error exit codes.

## MCP structured result compatibility

`search_symbols`, `get_callers`, `get_callees`, and `get_impact` advertise a
closed output schema and return `compass.code_context.v1`. The envelope carries
repository and generation identity, evidence-scoped freshness, evidence and
confidence summaries, truncation state, and warnings. Its `data` field is the
unchanged `compass.query/1` response. MCP `resultType: "complete"` remains the
protocol-level discriminator and is not overloaded with a Compass schema name.
Strict `compass.graph/1` validation requires non-empty `sourceTreeDigest` and
`generationId` build identities, so a successful envelope always satisfies its
advertised non-empty identity fields.

This top-level shape is compatibility-sensitive. Consumers must reject an
unknown `compass.code_context` major version and must not infer freshness when
the envelope reports `unknown`. Other typed and legacy text tools retain their
existing result shapes until separately versioned. The text-result tools
`get_neighbors`, `get_community`, `god_nodes`, `graph_stats`, `shortest_path`,
`list_prs`, `get_pr_impact`, and `triage_prs`, plus explicit traversal text mode
on `query_graph`, are deprecated from 0.4.0. They remain callable with unchanged
names and output; no removal release is scheduled before typed replacements
ship and receive a separate compatibility review.

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

The additive `compass ask` command routes bounded natural-language questions
to the existing typed search, callers, callees, impact, or node-trail operation
and returns the same `compass.query/1` response contract. `compass query`
automatically uses that path for high-confidence questions against a current
typed graph. Generic or contradictory questions, historical `--at` queries,
and requests with `--traverse`, `--dfs`, `--context`, `--budget`, or `--page`
retain the established text-traversal behavior. Explicit typed query commands
remain available and unchanged; ambiguous questions never invent a direction
or select an arbitrary symbol.

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
