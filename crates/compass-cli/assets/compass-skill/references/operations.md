# Diagnostics and recovery

Load this reference for invalid graphs, performance questions, or partial
semantic artifacts.

## Validate graph structure

```bash
compass diagnose multigraph --graph compass-out/graph.json
compass diagnose multigraph --graph compass-out/graph.json --json
```

Use diagnostics before assuming a query bug. Preserve the input graph and write
repairs to a separate output unless the user explicitly approves replacement.
Use `--directed` or `--undirected` only when the intended graph semantics are
known. JSON output is better for automation; text output is better for a bounded
human review. Examples are diagnostic samples, not a complete repair plan.

## Measure query behavior

```bash
compass benchmark compass-out/graph.json
```

Benchmark output describes the current machine, graph, and build. Do not compare
numbers across different graphs or environments without stating those
differences.

## Check pending semantic work

```bash
compass check-update .
```

This command checks the pending marker created when watch mode observes semantic
media it cannot refresh deterministically. Empty output means no marker was
reported; it does not prove the graph contains every desired semantic fact.

## Recover semantic pipeline artifacts

```bash
compass cache-check FILES --root ROOT
compass merge-chunks CHUNK... --out semantic-new.json
compass merge-semantic --cached cached.json --new semantic-new.json --out semantic.json
```

These commands are for controlled pipeline work. Validate JSON, inspect skipped
chunks, and require a successful merge before replacing an authoritative
artifact. `cache-check` uses the selected root and prompt/deep-mode identity;
cache hits from a different extraction configuration must not be treated as
equivalent. Write merged output to a new file first.

For any unfamiliar recovery option, run `compass <command> --help`. Prefer a
normal `compass update` or `compass extract` when the build can be reproduced
cleanly.

## Bounded answers

Use `--budget N` to cap completed command output at N approximate tokens
(UTF-8 bytes divided by four). Typed query text is compact by default; add
`--verbose` or `--evidence` for audit detail. Preserve match and coverage caveats
when summarizing an answer. Native pages continue with `--cursor`; other
commands print `compass output ID --offset N --budget N` to read the saved
output without repeating the command. Saved output is worktree-local and
bounded to sixteen records. Streaming `watch`, `serve`, and JSONL sessions reject
this presentation control; noninteractive `init --yes` supports it.
