# Query-relevance judgments

`judged.json` is a reviewed synthetic corpus for the `compass.query-judgments/1`
contract. IDs are deliberately stable synthetic graph identities: questions and
grades are authored independently from a ranker's current output.

The 80-question corpus is metric-contract coverage, not an executable graph
baseline: its synthetic IDs intentionally do not name the compact test graph.
`relevance_qualification.rs` also owns a small reviewed executable subset over
the checked-in `tests/support` graph fixture. That subset derives and pins the
canonical graph digest at runtime, executes real `CodeQueryEngine` requests,
and compares JSON, store, and repeated store observations after removing the
non-deterministic timing field. Do not derive its judgments or thresholds from
the current query result.

Add a question by assigning a unique ID, selecting its query class, recording
the intended operation/slots, and judging stable node, edge, or path identities.
For a legitimate no-answer query, include a concise reviewer rationale. Do not
derive expected IDs from the result being qualified; a second reviewer should
confirm new judgments before they become a baseline.
