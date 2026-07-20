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

Run the native test and compatibility suites from this directory:

```bash
cargo test --workspace
cargo clippy --workspace --all-targets --all-features -- -D warnings
```

The compatibility tests use the Python checkout at the repository root as the
behavioral oracle. Set `GRAPHIFY_PYTHON` when its interpreter is not located at
`.venv/bin/python` or `.venv/Scripts/python.exe`.
