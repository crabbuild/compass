# Code Graph v1 release-blocker disposition

Date: 2026-07-28

The final branch review reported one critical and eight important findings.
All nine were accepted; none were ignored.

| Severity | Finding | Disposition |
| --- | --- | --- |
| Critical | Authoritative artifacts could be published independently | Fixed by generation-scoped staging, artifact sealing, and one atomic active-generation pointer switch |
| Important | Symbol IDs included byte offsets | Fixed; stable identity uses semantic ownership and overload/signature discriminators |
| Important | Incremental projection lost typed fields and mixed AST/semantic evidence | Fixed with lossless typed records and non-AST evidence preservation |
| Important | History roots stored compatibility projections | Fixed; v1 history node and edge roots retain complete typed records |
| Important | Edge endpoint validation was incomplete | Fixed with an exhaustive relationship-kind matrix |
| Important | File inventory was inferred only from emitted facts | Fixed; detection and extraction outcomes publish zero-fact and non-complete records |
| Important | Query traversal could exceed node or edge budgets | Fixed; budgets are enforced before publication and adjacency is precomputed |
| Important | Source retrieval followed unsafe paths and read whole files | Fixed with confined, no-follow, streaming reads |
| Important | MCP returned stringified typed responses and errors | Fixed with structured content, concise text, and protocol errors |

Ignored-findings ledger: empty.
