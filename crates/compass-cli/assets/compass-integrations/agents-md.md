## compass

Use Compass as the local context layer for coding assistants.

Setup and synchronization:

1. Run `compass init` once to select repository scope.
2. Run `compass install` to install the detected assistant integration.
3. Keep `compass watch` running in a second terminal while you work.
4. If watch is not running or reports a failure, run `compass update .` after code changes and report the failed refresh.

Daily workflow:

- For a focused task, run `compass query "<question>"` before broad source search.
- For a first session or broad repository orientation, read only the bounded
  Agent Orientation at the start of `compass-out/GRAPH_REPORT.md`, then run a
  focused query.
- Inspect direction, ambiguity, graph completeness, domain truncation, and the
  final Pagination line before relying on a result.
- When a seed is ambiguous, repeat the query with the exact node ID.
- Follow `next=<cursor>` with the unchanged question and options plus
  `--cursor <cursor>` when the requested scope must be exhaustive; stop at
  `next=none`.
- Open only the cited source needed to verify decisive claims.
- Treat missing paths, inferred edges, and partial results as uncertain
  evidence, not proof.
- Keep explicit graph, revision, scope, provider, and output selections
  unchanged.
