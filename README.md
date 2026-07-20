# Trail graph engine

Trail is the native Rust implementation of Graphify. Graph commands live under
the `trail graph` namespace; the workspace also builds a `graphify` compatibility
executable for commands that have passed differential parity testing.

Currently ported:

```text
trail graph query
trail graph path
trail graph explain
trail graph affected
```

The internal `trail-files` crate also implements the compatibility-certified
deterministic filesystem contract used by future build commands: discovery and
classification, nested ignore rules, sensitive-file filtering, portable salted
hashes, stat caching, AST/semantic caches, incremental manifests, text slicing,
atomic writes, and interrupted-build guards. It is intentionally not surfaced
as a CLI command until the native extraction and graph-build pipeline is ready.

Run the native test and compatibility suites from this directory:

```bash
cargo test --workspace
cargo clippy --workspace --all-targets --all-features -- -D warnings
```

The compatibility tests use the Python checkout at the repository root as the
behavioral oracle. Set `GRAPHIFY_PYTHON` when its interpreter is not located at
`.venv/bin/python` or `.venv/Scripts/python.exe`.
