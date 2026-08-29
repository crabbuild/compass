# Shared-file evidence for C-014 review

The phase is implemented in one cumulative dirty working tree. `Cargo.toml`,
`Cargo.lock`, and `CHANGELOG.md` also contain accepted changes from C-004,
C-005, C-007, C-010, C-016, and C-017. Including their entire `HEAD` diff in a
C-014 packet falsely attributes those changes to this projection. The scoped
review therefore represents C-014's exact shared-file deltas here while the
deterministic and build gates continue to evaluate the real files.

## Exact C-014 shared-file deltas

`Cargo.toml` adds only:

```toml
# workspace member
"crates/compass-graphdb-surreal",

# workspace dependency
surrealdb = { version = "=3.2.4", default-features = false }
```

`Cargo.lock` adds the `compass-graphdb-surreal` package and the exact optional
dependency closure resolved by its three engine features. The relevant root is:

```text
name = "compass-graphdb-surreal"
version = "0.3.7"
dependencies = ["compass-model", "serde", "serde_json", "sha2", "surrealdb", "thiserror", "tokio"]
```

The resolved SDK package is exactly `surrealdb 3.2.4`. Default-feature `cargo
tree` checks prove no Surreal package reaches `compass-cli`, `compass-mcp`,
`compass-core`, or the projection crate without an engine feature.

`CHANGELOG.md` adds only the first Unreleased entry, which describes the new
optional projection crate, exact 3.2.4 pin, engine profiles, semantic round trip,
default-build isolation, and retained BSL obligations. Later entries in that
section predate C-014 and belong to their separately reviewed changes.

`docs/README.md` adds only the optional Surreal graph projection row. The
adjacent MCP conformance row belongs to C-010 and is retained in the cumulative
working tree without being attributed to C-014.

## Mechanical verification

The actual shared files were evaluated by all of these successful gates after
the final C-014 implementation edit:

- `cargo clippy --workspace --lib --bins --locked -- -D warnings`
- `cargo test --workspace --lib --bins --locked`
- default, Mem, SurrealKV, and RocksDB checks for `compass-graphdb-surreal`
- `sh scripts/check_product_boundary.sh`
- `sh scripts/check_surreal_feature_isolation.sh --binary`
- `openspec validate --all --strict`
- `git diff --check`

The C-004 2 GiB store publication default is not the projection default. C-014
uses `compass_model::DEFAULT_GRAPH_SIZE_CAP_BYTES`, the canonical
`GraphDocument` reader's 1 GiB bound, as an independent serialized projection
ceiling. It does not claim that maximum node and relation counts are jointly
attainable beneath that byte ceiling; C-015 measures the ratified corpora.
