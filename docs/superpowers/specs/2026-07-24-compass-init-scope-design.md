# Compass Init and Persisted Build Scope Design

**Date:** 2026-07-24
**Status:** Approved for implementation planning

## Summary

Compass can build a project with `compass update` and can exclude additional
paths with repeatable `--exclude` options, but it has no first-run setup
command and no repository-owned build scope. Users must remember the correct
root and exclusions on every update or watch invocation, and they cannot
express a persisted positive scope such as “only these services, source
directories, and manifest files.”

This feature adds:

```bash
compass init [PATH]
```

The command interactively configures a project, previews the selected corpus,
writes a reviewable `.compass/config.toml`, and performs a forced initial
structural rebuild. Repeatable flags provide the same workflow for scripts and
continuous integration. The saved scope is automatically reused by
`compass update` and `compass watch`.

## Goals

- Give first-time users one discoverable command that configures and builds a
  Compass graph.
- Let users include literal files, literal directories, and project-relative
  glob patterns.
- Persist project scope in a small, reviewable repository file.
- Apply identical scope semantics to initialization, updates, and filesystem
  watching.
- Preview and validate the effective corpus before writing configuration.
- Preserve existing Git-ignore behavior, built-in safety skips, output
  atomicity, and deterministic structural extraction.
- Support non-interactive automation without maintaining a separate setup
  path.

## Non-goals

- Configuring semantic providers, credentials, history, hooks, or assistant
  integrations.
- Persisting every `update` or `extract` option.
- Adding general-purpose `config get`, `config set`, or `scope` subcommands.
- Allowing absolute paths or sources outside the configured project root.
- Overriding Compass's built-in safety skips through an include rule.
- Changing Graphify compatibility-frontend behavior.
- Running multiple independent builds and merging their graphs.

## CLI contract

### Canonical form

```text
compass init [PATH]
  [--include <PATH_OR_GLOB>]...
  [--exclude <GLOB>]...
  [--yes]
  [--force]
```

`PATH` is the project root and defaults to the current directory. Include and
exclude values are repeatable. Both split and inline option forms are accepted:

```bash
compass init . --include src --include='services/*/src'
```

Examples:

```bash
# Interactive first-run setup.
compass init

# Reconfigure an existing project interactively.
compass init . --force

# Fully local automation with a custom scope.
compass init . \
  --include src \
  --include 'services/*/src' \
  --include Cargo.toml \
  --exclude '**/generated/**' \
  --yes

# Build the complete repository without prompts.
compass init . --yes
```

`--yes` selects non-interactive operation and accepts the validated preview.
Without `--yes`, Compass requires interactive standard input and prompts before
writing. A non-terminal invocation without `--yes` is a usage error that
explains how to run non-interactively.

`--force` permits replacement of an existing `.compass/config.toml`. Without
it, `init` refuses to overwrite the file in both interactive and
non-interactive modes. This protects checked-in project configuration from an
accidental local rewrite.

The command is Compass-native. `graphify init` remains unknown.

## Interactive experience

Interactive setup uses ordinary terminal prompts rather than a full-screen
terminal interface:

1. Show the resolved project root.
2. Ask whether to scan the whole repository or customize the scope.
3. For custom scope, accept zero or more include entries. Each entry may be a
   file, directory, or glob.
4. Accept zero or more exclusion globs and suggest detected common
   generated/vendor directories without selecting them silently.
5. Validate the rules and show the effective file count grouped by detected
   corpus type.
6. Show the target configuration and output paths.
7. Ask for confirmation.
8. Atomically write the configuration and run the initial rebuild.

Command-line include and exclude values pre-populate the interactive answers.
The user may review them before confirming. Cancelling or answering “no” exits
without writing configuration or starting a build.

An empty include list has one explicit meaning: include the complete eligible
repository. It is not an empty corpus.

## Configuration contract

The generated file is:

```toml
version = 1

[build]
include = ["src/", "services/*/src", "Cargo.toml"]
exclude = ["vendor/**", "**/generated/**"]
```

The file deliberately stores only project scope:

- `version` is required and must equal `1`.
- `build.include` is an optional array of unique strings. Missing or empty
  means all eligible files below the project root.
- `build.exclude` is an optional array of unique strings. Missing or empty
  means no additional configured exclusions.

Serialization is deterministic: keys use the order above, entries retain the
first user-supplied occurrence after normalization, normalized duplicates are
removed, paths use `/` separators on every platform, and the file ends with
one newline. Compass rejects unknown keys, unknown sections, unsupported
versions, non-string entries, and invalid TOML with a diagnostic that names
`.compass/config.toml`.

The `.compass` directory remains outside the detected corpus through the
existing built-in directory skip. The configuration is safe to commit and
contains no credentials, machine-specific absolute paths, or generated
timestamps.

## Scope semantics

All matching operates on normalized, project-root-relative paths with `/`
separators.

An include entry is interpreted as follows:

- A literal file includes that file.
- A literal directory includes every otherwise eligible descendant.
- An entry containing glob metacharacters matches files and directories using
  Compass's documented project-relative glob syntax. A matching directory
  includes its eligible descendants.

Every include supplied to `init` must match at least one eligible file at setup
time. This catches misspellings while still allowing a glob to match future
files inside a currently matched directory. The final effective corpus must
contain at least one supported file.

Absolute paths, `..` traversal that escapes the project root, empty strings,
malformed globs, and paths that resolve outside the root through symlinks are
rejected.

Discovery applies filters in this order:

1. Built-in safety skips, including VCS metadata, dependency caches, build
   output, Compass output, and `.compass`.
2. Git ignore rules unless `--no-gitignore` is active on the consuming command.
3. Configured includes; an empty list admits every remaining eligible path.
4. Configured excludes.
5. command-line `--exclude` rules on `update` or `watch`.

Later filters only remove paths. Includes do not resurrect a built-in skipped,
Git-ignored, configured-excluded, or command-line-excluded path. This makes the
precedence safe and predictable.

## Initialization data flow

`compass init` follows one transaction boundary for configuration and the
existing transaction boundary for graph outputs:

1. Parse arguments and resolve/canonicalize the project root.
2. Refuse an existing configuration unless `--force` is present.
3. Collect interactive or non-interactive scope inputs.
4. Normalize and validate a candidate `ProjectConfig`.
5. Run detection with the candidate scope and render the preview.
6. Confirm unless `--yes` is present.
7. Atomically create `.compass/config.toml`.
8. Invoke the normal Compass structural update path with `force = true`.
9. Report the saved configuration, effective scope count, and output
   directory.

The initial build is equivalent to a forced `compass update` for the resolved
root. It is local and deterministic; `init` does not select or invoke a
semantic provider.

The configuration is intentionally retained if the build fails. The failure
diagnostic explains that setup was saved and can be retried with
`compass update`. Existing incomplete-build guards and atomic publication
continue to protect graph artifacts.

If configuration writing fails, no build starts. If the user cancels, neither
configuration nor build outputs are changed.

## Reuse by update and watch

After root resolution, Compass looks for exactly:

```text
<resolved-project-root>/.compass/config.toml
```

It does not walk unrelated parent directories looking for configuration.
`compass update` without a positional root keeps its existing saved-root
behavior; once that root is resolved, its project configuration is loaded.

The loader runs before discovery and converts the saved include/exclude rules
into the same shared scope model used by `init`. A malformed or unsupported
configuration is a hard error. Compass never silently widens the corpus when
the configuration cannot be read.

`compass watch` creates its immutable event filter from the same resolved
scope. Events outside the includes or inside an exclusion do not schedule a
rebuild. Newly created files that match the saved rules do schedule one.
Every rebuild also uses the same scope model, so event filtering and actual
graph contents cannot drift.

Explicit command-line exclusions are additive and ephemeral. There is no
`--include` override on `update` or `watch` in this feature; users change the
positive project scope by editing the reviewable file or rerunning
`compass init --force`.

## Architecture

### `compass-files`: project configuration

A focused configuration module owns:

- the versioned `ProjectConfig` and `BuildScope` data structures;
- strict TOML parsing;
- deterministic TOML rendering;
- path and glob normalization;
- candidate validation;
- atomic load/write helpers.

The public API returns typed errors that distinguish unreadable files, invalid
syntax, unsupported versions, unknown fields, invalid patterns, root escapes,
and unmatched initialization rules.

Configuration parsing belongs beside file discovery because the schema
describes corpus selection and must be reusable without depending on the CLI.

### `compass-files`: include-aware discovery

`DetectOptions` gains a positive-scope field in addition to the existing
`extra_excludes`. Detection compiles include and exclude rules once per scan.
Directory walking prunes a subtree only when the matcher can prove no include
could match below it; otherwise it walks the subtree and filters eligible
files. This preserves glob correctness while avoiding unnecessary traversal
for literal scopes.

`WatchPathFilter` receives the same compiled scope and precedence. The matching
implementation is shared with normal detection rather than reimplemented in
watcher code.

### `compass-cli`: init orchestration

A new `init_commands.rs` module owns init argument parsing, prompt flow,
preview rendering, overwrite/cancellation behavior, and final reporting.
Interactive input/output are injected behind small reader/writer interfaces so
tests do not depend on a real terminal.

Because the current library dispatcher returns a completed `Outcome`, the
binary routes interactive `init` through a streaming entry point, as it already
does for other commands that cannot be represented as a single pure outcome.
The non-interactive path calls the same orchestration with confirmation
disabled; it is not a second implementation.

The build command parser and executor are separated only as far as needed for
`init` to invoke a typed forced update without constructing a second command
line. Existing update behavior remains the source of truth.

### Build and watch integration

The normal build and watch option assembly loads project configuration after
the root is known, then passes its `BuildScope` into `BuildOptions` or
`WatchOptions`. Graphify compatibility mode does not load the new Compass
configuration.

Help metadata, shell completions, the command reference, configuration
reference, and getting-started documentation gain the new command and scope
contract.

## Errors and exit status

`init` uses existing Compass conventions:

- `0`: configuration was written and the initial rebuild completed, or the
  user cancelled before any mutation;
- `1`: filesystem, configuration-write, detection, or build runtime failure;
- `2`: invalid options, invalid scope entries, non-terminal use without
  `--yes`, existing configuration without `--force`, or an empty/unmatched
  requested scope.

Diagnostics identify the failing option or configuration entry and the
resolved project root. They do not dump entire corpora or environment values.

If a build fails after configuration is saved, the message distinguishes the
two states:

```text
Compass configuration saved to .compass/config.toml.
Initial build failed: <bounded diagnostic>
Fix the reported issue, then run `compass update`.
```

## Testing strategy

Implementation follows red-green-refactor slices.

### Configuration tests

- Parse and deterministically render the version-1 schema.
- Reject unsupported versions, unknown keys, wrong types, and malformed TOML;
  collapse duplicate entries after normalization while preserving first-seen
  order.
- Normalize Windows separators to `/`.
- Reject absolute paths, root escapes, escaping symlinks, empty entries, and
  malformed globs.
- Prove atomic replacement and preservation on write failure.

### Discovery tests

- Include one literal file.
- Include all eligible descendants of a literal directory.
- Include multiple scopes and a project-relative glob.
- Treat an empty include list as the complete eligible repository.
- Apply built-in skips, Git ignores, configured excludes, and CLI excludes in
  the specified order.
- Reject unmatched initialization rules and an empty final corpus.
- Keep matching paths correct when a literal directory and glob overlap.
- Verify the normal and watcher matchers make the same decision for every test
  path.

### CLI tests

- `compass init --yes` writes the default whole-repository configuration and
  performs a forced build.
- Repeatable includes/excludes write the expected file and constrain the
  initial graph.
- Interactive answers produce the same configuration as equivalent flags.
- Cancellation writes nothing and starts no build.
- Existing configuration requires `--force`.
- Non-terminal use requires `--yes`.
- Invalid/unmatched rules fail before writing.
- A build failure retains the valid configuration and reports the retry path.
- `graphify init` remains unsupported.

### Update and watch tests

- `compass update` automatically reuses saved includes and excludes.
- Saved-root update resolution loads configuration from the resolved root.
- CLI exclusions remain additive.
- Invalid saved configuration stops rather than widening the graph.
- Watch events outside scope are ignored.
- A newly created matching file triggers a rebuild and enters the graph.
- The full existing build, incremental, watch, help, and compatibility suites
  remain green.

## Completion criteria

The feature is complete when:

- interactive and non-interactive initialization produce the same validated
  configuration model;
- `.compass/config.toml` safely persists file, directory, and glob scope;
- the initial forced structural build contains only the configured corpus;
- `update` and `watch` automatically and consistently reuse that scope;
- invalid configuration cannot silently widen a build;
- help, completions, and documentation describe the workflow;
- Graphify compatibility behavior is unchanged; and
- the Compass workspace test suite passes.
