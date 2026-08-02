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

Legacy Graphify names are intentionally unsupported. Existing Graphify state
must be archived or removed before creating fresh Compass artifacts. See
[`MIGRATION.md`](MIGRATION.md) for the hard-cutover procedure.

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

The current local build publishes `graph.json` (`compass.graph/1`), a validated
`compass-store.sqlite3` sidecar, and a typed `store.ref` selector. The store is
selected by typed code queries by default when the selector and sidecar agree,
while `graph.json` remains a complete compatible engine; explicit JSON
selection never requires opening a database. The SQLite file and reference are
internal realizations of the backend-neutral `compass-store` contract, not a
stable SQL schema or pointer format that consumers may query directly.

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
