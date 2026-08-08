# Query-relevance judgments

`judged.json` is a reviewed synthetic corpus for the `compass.query-judgments/1`
contract. IDs are deliberately stable synthetic graph identities: questions and
grades are authored independently from a ranker's current output.

The 80-question corpus is metric-contract coverage, not an executable graph
baseline: its synthetic IDs intentionally do not name the compact test graph.
`relevance_qualification.rs` also owns 23 reviewed, production-shaped
executable questions over the checked-in `tests/support` graph fixture. They
cover exact and normalized symbol search, typo recall, production-versus-test
ambiguity, callers, callees, impact, path paraphrases, domain vocabulary, and
no-answer behavior. The subset derives and pins the canonical graph digest at
runtime, executes real `CodeQueryEngine` requests, and compares JSON, store,
and repeated store observations after removing the non-deterministic timing
field. Do not derive its judgments or thresholds from the current query
result.

Add a question by assigning a unique ID, selecting its query class, recording
the intended operation/slots, and judging stable node, edge, or path identities.
For a legitimate no-answer query, include a concise reviewer rationale. Do not
derive expected IDs from the result being qualified; a second reviewer should
confirm new judgments before they become a baseline.

## Expanding from production queries

Compass does not send query telemetry over the network. An approved operator
can opt in to bounded local MCP `query_graph` logging by setting
`COMPASS_QUERY_LOG` to a protected path. Logging stops at 16 MiB, captures both
typed natural queries and compatibility traversal queries, and does not include
typed responses. Compatibility-traversal response capture has a separate
`COMPASS_QUERY_LOG_RESPONSES` switch; do not enable it for relevance sampling.
An operator can also
provide another approved local JSONL export whose records contain a string
`question`. Prepare either source as a bounded review queue:

```bash
python3 scripts/prepare_query_relevance_review.py \
  --input /approved/private/queries.jsonl \
  --output /approved/private/query-review.json
```

The importer rejects inputs above 16 MiB, more than 10,000 JSONL lines, records
above 64 KiB, and questions above 4,096 bytes. It applies best-effort redaction
for paths, URLs, email addresses, assigned secrets, JWTs, and long tokens;
deduplicates deterministically; and emits at most 256
`compass.query-review-candidates/1` records. It deliberately omits responses,
corpus paths, timestamps, and expected answers. The qualification gate runs
the importer's deterministic self-test.

Redaction is not anonymization: symbol names and business vocabulary can still
be sensitive. Keep raw and prepared files outside the repository, review them
under the source repository's access policy, and apply an explicit retention
period. For every accepted candidate, a reviewer must select a pinned graph
revision/digest and author the expected intent, slots, node/edge/path IDs,
acceptable ambiguity, forbidden results, and no-answer rationale. A second
reviewer must confirm those judgments before the case is copied into the
executable corpus. Never auto-promote importer output or infer judgments from
the current ranker.
