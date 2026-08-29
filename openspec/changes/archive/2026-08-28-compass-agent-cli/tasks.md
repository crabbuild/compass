## 1. Namespace and Contracts

- [x] 1.1 Add `agent` routing and nested help for all six public subcommands; verify CLI help and invalid-subcommand contract tests pass
- [x] 1.2 Add deterministic typed agent inventory and MCP configuration rendering; verify text, versioned JSON, Codex TOML, Claude JSON, OpenCode JSON, and generic JSON snapshots
- [x] 1.3 Delegate `agent install` to the managed installer without translating arguments; verify success and failure bytes, side effects, and exit codes match legacy `compass install`

## 2. Portable Bundle Operations

- [x] 2.1 Expose narrow immutable installer helpers for registry metadata, embedded seven-skill assets, managed destinations, and checksum verification; verify installer tests preserve registry and manifest behavior
- [x] 2.2 Implement same-filesystem staged export with deterministic checksums and cleanup; verify repeat exports are byte-identical and non-empty destinations remain untouched
- [x] 2.3 Implement bounded, non-executing bundle validation with containment, symlink, checksum, absolute-path, and redacted credential checks; verify valid HTTPS URLs pass while unsafe fixtures fail deterministically

## 3. Diagnostics and Public Evidence

- [x] 3.1 Implement offline aggregate doctor checks for binary version, MCP protocol, graph presence/freshness, installed skill checksums, and platform configuration; verify healthy, stale-graph, bad-checksum, and missing-config cases and exit statuses
- [x] 3.2 Add public CLI integration tests for all six subcommands, JSON schemas, deterministic ordering, invalid arguments, and compatibility behavior; verify the focused `compass-cli` test targets pass
- [x] 3.3 Update command/reference documentation, compatibility notes, and changelog; verify documented commands and schema identifiers match CLI help and fixtures

## 4. Qualification and Completion

- [x] 4.1 Run formatting, focused tests, all-target/all-feature `compass-cli` Clippy, product-boundary checks, strict OpenSpec validation, and relevant workspace baselines; record exact results
- [x] 4.2 Refresh the Compass graph, perform artifact refinement and mandatory adversarial review, resolve all blocking findings, synchronize the capability spec, and archive the verified OpenSpec change
