# Concept search qualification

Native concept recall reached the expected module among the first three
production source paths for **26 / 30 questions (86.7%)** on a freshly rebuilt pinned Dify graph.
The first path was correct for **16 / 30 questions (53.3%)**. This is module
retrieval evidence; it does not establish implementation equivalence or runtime
behavior, and approximate matches remain labelled as such.

Input: [Dify at `1a918ea407990ecd2156af2614932db8c9c43091`](https://github.com/langgenius/dify/tree/1a918ea407990ecd2156af2614932db8c9c43091/api).
The checkout was read-only. The 30 novice-style questions were agent-authored;
the source-derived expected modules were fixed before the first query run.
They are not an independent human evaluation or a competitor comparison.
The checked-in [corpus](../../tests/qualification/concept-search-dify.json)
contains every question, expected path prefix, and the 24/30 acceptance threshold.
The [machine report](concept-search-qualification.json) retains each returned
module path, correctness judgment and truncation flag.

## Measurements

The criterion takes the first three distinct production source paths in native
result order, excluding tests, fixtures and mocks. A path must match a pinned
expected prefix. Nonempty results alone do not count as correct. First-path
accuracy is reported separately so the top-three criterion does not conceal
ranking limitations.

The initial implementation found 4/30 expected modules. Ranking only the first
64 ID-ordered candidates improved this to 12/30, but still lost implementation
code behind parameters and data types. Bounded posting recall followed by
context, filename, implementation-kind and acronym-aware ranking reached 26/30.
No questions or expected modules were changed between these runs. A fresh
AST-v15 rebuild with retained rationale prose reproduced 26/30 and 16/30.

Four questions still miss their expected module in the first three paths:

- Background jobs: scheduler utilities and services outrank task execution.
- Database transactions: session factories outrank the expected database extension
  or repositories.
- File uploads: utilities and plugin upload adapters outrank the general file
  service/factory.
- Subscription payments: event-subscription code outranks the financial modules.

Every answer disclosed candidate/response truncation under 32 retained nodes,
64 candidates per alternative, depth one and 1,000 relationship records.
Responses also disclose the published graph's partial coverage. The graph used native code-only extraction, maximum inference,
excluded `tests/**`, and omitted clustering and visualization. Optional semantic
search was disabled for this public-repository run. These results do not claim
performance, token-cost parity, transitive completeness or precise structural
resolution for approximate matches.

## Native regression evidence

`concept_search` tests check 30 business questions against intended declarations,
strict lookup, deterministic repeated responses, scoped discovery, and ranking
with distracting parameters. A fresh Markdown publication test puts the concept
past the short display label and follows only witnessed references to code,
with JSON/store parity and no fabricated missing-symbol target.

The optional semantic fixture learns TF-IDF/truncated-SVD embeddings from a small
local corpus. It checks a vocabulary bridge, disconnected and unknown terms,
explicit opt-in, cache reuse, and resetting a cached engine to the default.
Unit tests cover deadline and vocabulary-limit errors. No remote model, credentials,
network service or vector database is used. Large corpora fail with a typed limit
error; this is not qualification of semantic quality on all of Dify.

CLI and MCP regressions verify approximate provenance and strict lookup. Rust and
viewer decoders enforce the same 4 KiB UTF-8 prose boundary. The FTS cache test
preserves a marker across reopen, proving a valid index was reused rather than
silently rebuilt during validation.

## Reproduce

Set `CARGO_TARGET_DIR` to this checkout's external target directory as required by
`AGENTS.md`, `DIFY_API` to the pinned checkout's `api/`, and `QUALIFICATION_OUT` to
an artifact directory outside the source checkout.

```bash
CARGO_TARGET_DIR="$CARGO_TARGET_DIR" cargo build --release --locked \
  -p compass-core --example republish_qualification \
  -p compass-query --example query_batch
"$CARGO_TARGET_DIR/release/examples/republish_qualification" \
  "$DIFY_API" "$QUALIFICATION_OUT"
python3 scripts/qualify_concept_search.py \
  tests/qualification/concept-search-dify.json > "$QUALIFICATION_OUT/requests.json"
"$CARGO_TARGET_DIR/release/examples/query_batch" \
  "$QUALIFICATION_OUT/compass-out/graph.json" \
  "$QUALIFICATION_OUT/requests.json" "$QUALIFICATION_OUT/responses.json" \
  "$QUALIFICATION_OUT/query-cache"
python3 scripts/qualify_concept_search.py \
  tests/qualification/concept-search-dify.json \
  --responses "$QUALIFICATION_OUT/responses.json" > "$QUALIFICATION_OUT/report.json"
```

The batch helper publishes its bounded response artifact atomically only after
all queries succeed. The evaluator exits unsuccessfully below the pinned
threshold. Source checkout and generated graph contents are not committed.
