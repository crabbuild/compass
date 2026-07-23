# Make Compass command help descriptive and comfortable

This specification defines a complete help experience for the native `compass` command. It covers every public command and user-facing nested command without changing command execution, machine-readable output, exit codes, or the Graphify compatibility frontend.

## Goal and audience

The change helps first-time and returning Compass users discover commands, understand options, and recover from command-name mistakes without opening the README. A successful implementation lets a user answer three questions from the terminal: what a command does, how to invoke it, and which options matter for their task.

The implementation must:

- Give every public command a concise summary in root help
- Give every public command and user-facing nested command a dedicated help page
- Explain positional arguments, options, accepted values, defaults, conflicts, and repeatability where applicable
- Include runnable examples and relevant workflow tips
- Support color on interactive terminals without adding control characters to redirected output
- Preserve the Graphify compatibility frontend

The work does not redesign command parsing, normal result output, machine-readable formats, or runtime diagnostics.

## Help architecture

Add a dedicated help module to `compass-cli`. The module owns typed page metadata, command-path lookup, typo suggestions, plain rendering, and styled rendering. Existing command modules continue to own parsing and execution.

Each help page uses a data structure with these fields:

- Command path and one-line summary
- Long description when the summary cannot explain the command fully
- One or more usage forms
- Positional arguments
- Options with value names, descriptions, accepted values, defaults, conflicts, requirements, and repeatability
- Nested commands
- Examples
- Tips or operational notes
- Visibility, either public or internal

A single registry supplies root listings and path lookup. Root help must not maintain a second command-name list. Command modules may declare their detailed page metadata near the parser, but they register pages through the common catalog.

Internal process commands, including history workers and hook subprocesses, remain absent from root listings. An explicit help request for an internal command returns a page that labels the command as internal and directs users to the public parent workflow.

## Help routes

Compass resolves the complete command path before rendering help. These forms return the same page:

```text
compass history build --help
compass history build -h
compass help history build
```

`compass help` and `compass --help` return root help. If a help path contains an unknown segment, Compass reports the unknown name, suggests one sufficiently close sibling, and points to the nearest valid help page.

The help router must not alter Graphify behavior. `graphify --help`, Graphify command help, and Graphify parsing retain their current output and routing.

## Root help layout

Root help starts with a short product description and usage line. It groups public commands by the task a user wants to perform instead of presenting one undifferentiated list. The groups and their commands are:

- Build and maintain: `update`, `extract`, `watch`, `cluster-only`, `label`, `merge-graphs`, `cache-check`, `merge-chunks`, and `merge-semantic`
- Explore: `query`, `path`, `explain`, `affected`, and `benchmark`
- History: `history` and `diff`
- Visualize and export: `tree` and `export`
- Integrate and automate: `serve`, `global`, `clone`, `add`, `prs`, `hook`, `install`, `uninstall`, `provider`, `save-result`, and `reflect`
- Diagnose and support: `diagnose`, `check-update`, `merge-driver`, `hook-check`, and `hook-guard`

Every row contains a command name and one-line summary. Nested command names do not appear as separate root commands. Root help ends with global options and a hint for `compass help <command>`.

The plain-text shape follows this example:

```text
Compass: turn a codebase into a searchable knowledge graph

Usage: compass <COMMAND> [OPTIONS]

Build and maintain:
  update          Incrementally refresh the local graph
  extract         Build a graph with optional semantic extraction
  watch           Rebuild automatically when project files change

Explore:
  query           Search the graph with natural language or CompassQL
  path            Find the shortest path between two nodes
  explain         Explain a node and its relationships

Run `compass help <command>` for detailed help.
```

Every public command must appear exactly once in these groups. The root renders `diagnose` as a command; its `multigraph` operation appears on the detailed page.

## Command help layout

Every command page uses the same section order:

1. Summary and optional description
2. Usage
3. Commands, when the page owns nested commands
4. Arguments
5. Options
6. Examples
7. Tips or operational notes

The renderer omits empty sections. It aligns term descriptions within a section and wraps descriptions to a stable maximum width. Continuation lines align with the description, not the option name. The plain renderer produces deterministic output for tests, pipes, and documentation captures.

Examples must use valid current syntax and show distinct tasks. Most leaf commands need two examples: the default workflow and one useful variation. Commands with destructive or credential-sensitive behavior must include the relevant warning or prerequisite as a note.

Tips connect adjacent workflows only when the relationship helps a user choose their next command. For example, `update` can mention `watch`, while `query` can mention `path` for relationship tracing. Tips must not advertise unrelated features.

## Terminal styling

Styled help uses American National Standards Institute (ANSI) colors for headings, command names, value placeholders, and tips. Color never carries information that plain text omits.

The `compass` binary enables styled help only when standard output is an interactive terminal and neither condition below applies:

- `NO_COLOR` exists in the environment
- `TERM` equals `dumb`, case-insensitively

Redirected output, library-level help rendering, and tests use plain text. The implementation uses `std::io::IsTerminal` plus a small semantic style layer. It does not add a command-line framework solely for styling.

## Usage errors and suggestions

Unknown top-level commands and unknown help-path segments compare against visible sibling names. Compass prints one suggestion only when the edit distance passes a conservative threshold. A distant name receives no suggestion.

Usage failures point to the most specific available page:

```text
Run `compass history build --help` for usage.
```

The implementation adds this hint only to failures that the current parser identifies as usage errors. Runtime, storage, provider, network, and graph-validation failures must not gain a help hint. Existing exit codes remain unchanged.

This feature does not require fuzzy matching for every option parser. Detailed option descriptions and accepted values provide the primary option guidance. A command may suggest a close option only when its parser already distinguishes unknown options from other failures.

## Coverage

Full coverage means:

- Every public command in the Compass dispatcher appears once in root help
- Every public nested command has an addressable help page
- Every accepted public option appears on its owning page
- Every page identifies defaults and accepted values that affect behavior
- Every leaf page includes at least one valid example
- Hidden internal commands do not appear in public listings
- Explicit internal-command help explains its status and public alternative

The required nested page families are:

- `history`: `enable`, `disable`, `status`, `build`, `rebuild`, `list`, `show`, `prefer`, `export`, and `gc`
- `export`: `html`, `callflow-html`, `obsidian`, `wiki`, `svg`, `graphml`, `neo4j`, and `falkordb`
- `provider`: `add`, `list`, `show`, and `remove`
- `global`: `add`, `remove`, `list`, and `path`
- `hook`: `install`, `uninstall`, and `status`
- `diagnose`: `multigraph`

Compatibility aliases and platform shortcuts may be grouped under their owning public command when listing each alias would overwhelm root help. Their accepted spelling must still appear on the detailed page.

## Testing strategy

Implement the help system with test-driven development. Add one failing behavior test before each implementation slice.

Structural tests inspect the registry and require:

- Unique command paths
- One root placement for every public top-level command
- Non-empty summaries and usage forms
- At least one example on every public leaf page
- Reachable parent pages for every nested page
- No public child beneath an internal-only parent

Rendering tests verify section order, alignment, wrapping, and omission of empty sections. Rendering the same page in plain and styled modes must produce identical text after stripping ANSI sequences. Plain output must contain no escape byte.

Routing tests cover root forms, command forms, nested forms, internal pages, unknown segments, close suggestions, and distant names. Environment-policy tests cover terminal output, redirected output, `NO_COLOR`, and `TERM=dumb` through an injectable style decision rather than a real terminal.

Regression tests capture the current Graphify compatibility help before the change and require byte-for-byte equality afterward. The existing CLI suite must continue to pass so the help work does not change parsing, result output, or exit codes.

## Completion criteria

The feature is complete when all public Compass commands meet the coverage rules, both help invocation styles resolve correctly, terminal styling follows the environment policy, typo recovery behaves conservatively, Graphify help remains unchanged, and the full Compass workspace test suite passes.
