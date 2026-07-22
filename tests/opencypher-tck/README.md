# CompassQL openCypher conformance corpus

This directory contains an intentionally selected, unmodified snapshot of
openCypher TCK feature files used to verify CompassQL's documented read-only
subset. The files come from openCypher tag `2024.3`, commit
`677cbafabb8c3c5eed458fd3b1ec0daec8d67d23`.

`manifest.toml` records every vendored file's SHA-256 and the scenarios that
Compass currently executes as conformance tests. A vendored feature file does
not imply that every scenario in that file is supported. Mutation scenarios
and unsupported language surface remain explicitly outside the read-only
subset.

Run the executable scenarios with:

```sh
cargo test -p compass-query --test opencypher_tck
python3 scripts/check_compassql_support.py
```

The feature files are Apache-2.0 licensed by the openCypher contributors. See
`LICENSE` and `THIRD_PARTY_NOTICES.md`. CompassQL is an independent
implementation and is not endorsed by Neo4j or the openCypher project.

To update the snapshot, fetch files only from a reviewed openCypher release,
retain each file byte-for-byte, update `tag`, `commit`, and every SHA-256 in
`manifest.toml`, classify supported and explicitly rejected scenario IDs, and
run both commands above. Reviewers must confirm that no newly selected step
depends on mutation, administration, ambient database state, or an unbounded
path before accepting the update.
