# Program IR qualification corpus

This corpus exercises the honest boundary between syntax evidence and resolved
semantic evidence. The Rust and TypeScript sources are intentionally valid
parser inputs with constructs that syntax-only analysis cannot fully resolve.
Their `expected.json` files describe required coverage and ambiguity rather
than snapshotting unstable implementation details.

The SCIP cases are generated in the `compass-program` and `compass-core` tests
so the binary protobuf inputs stay readable and reproducible. The checked-in
`scip/cases.json` maps each required case to its executable test, expected
freshness state, and failure or conflict behavior. `malformed.scip` is a
deliberately invalid discovery fixture.

Run the complete corpus and determinism matrix with:

```bash
bash scripts/qualify_program_ir.sh
```

The script copies source fixtures into temporary repositories. It never updates
tracked fixtures in place.
