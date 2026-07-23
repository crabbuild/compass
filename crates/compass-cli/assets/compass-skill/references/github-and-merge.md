# Repositories, pull requests, and merged graphs

Load this reference for repository URLs, multi-repository questions, pull
requests, or graph composition.

## Clone and build

```bash
compass clone https://github.com/OWNER/REPOSITORY
compass clone URL --branch BRANCH --out DIRECTORY
compass update DIRECTORY
```

Cloning uses the network and writes a new checkout. Resolve the destination
before running it and do not overwrite an existing directory.

## Compose graph data

```bash
compass merge-graphs graph-a.json graph-b.json --out merged.json
compass global add path/to/graph.json --as repository-name
compass global list
compass global path
```

Use `merge-graphs` for a concrete merged artifact. Use `global` when maintaining
the local cross-project registry. Preserve repository identity so same-named
symbols are not presented as one source.

## Pull-request workflows

```bash
compass prs
compass prs NUMBER
compass prs --worktrees
compass prs --conflicts
```

PR operations may call external Git hosting tools and read worktree state.
Graph-impact results show shared communities and likely review scope; they do
not prove merge conflicts. Run `compass prs --help` before triage, base-branch,
or mutating options.

`compass merge-driver` is intended for configured merge workflows. Do not invoke
it manually on user files without understanding the base/current/other contract.
