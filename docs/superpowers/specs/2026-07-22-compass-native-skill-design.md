# Install a native Compass skill

This design defines how `compass install` creates a new skill named `compass`. The skill may reuse Graphify’s workflows and platform knowledge, but every installed artifact must describe and invoke Compass.

## Goal and audience

Compass users can register a native knowledge-graph skill with their coding assistant by running one Compass command. Maintainers can update borrowed Graphify guidance without reintroducing Graphify branding, commands, paths, or runtime dependencies.

## Scope

The implementation will:

- Install a skill whose frontmatter name is `compass`
- Install the skill in each platform’s `compass` skill directory
- Use `compass`, `/compass`, `compass-out/`, and `COMPASS_*` throughout generated artifacts
- Reuse Graphify skill content when the described behavior exists in Compass
- Keep the current supported platform set
- Make install, reinstall, and uninstall idempotent
- Remove only Compass-owned artifacts during uninstall
- Remove `compass-out/` only when the user passes `--purge`

The implementation will not modify or remove an existing Graphify installation. It will not preserve Python Graphify as an installer oracle because native Compass output intentionally differs.

## Installation architecture

`compass install` remains the generic entry point. Direct platform commands such as `compass codex install` continue to call the same installer implementation.

The installer embeds one canonical Compass skill body plus a progressive
reference bundle for every platform. A platform record selects the destination;
all platforms receive the same native operating contract. Global installs
resolve the platform’s user configuration directory. Project installs resolve
the same structure under the current project.

Examples include:

| Platform | Project destination |
| --- | --- |
| Claude Code | `.claude/skills/compass/SKILL.md` |
| Codex | `.codex/skills/compass/SKILL.md` |
| Agent Skills compatible tools | `.agents/skills/compass/SKILL.md` |
| OpenCode | `.opencode/skills/compass/SKILL.md` |

Each installed skill directory contains `SKILL.md`, a `.compass_version`
ownership marker, and a `references/` directory. The core covers the existing
graph fast path, build selection, evidence rules, and completion contract.
Sidecars cover query and CompassQL, refresh, semantic extraction, immutable
history, hooks, watch and ingestion, exports, MCP serving, repository
composition, reflections, diagnostics, labeling, security boundaries, a full
public-command inventory, and graph provenance.

`tools/skillgen/` provides a native Rust build-time guard inspired by Graphify's
skill generator. Before assets are embedded, it validates frontmatter, minimum
content coverage, exact agreement between the core reference index and bundled
sidecars, deterministic discovery, headings, native-brand constraints, the exact
platform-integration asset set, and every command in
the rich-help catalog. Public help pages, CLI dispatch, and skill coverage must
agree; internal worker commands must keep an explicit do-not-invoke boundary.

## Borrowed skill content

Graphify skill files provide the source structure and operational lessons. The Compass port will audit each instruction against the current native command surface before copying it.

The conversion contract is:

- `name: graphify` becomes `name: compass`
- Graphify invocation examples become their native `compass` equivalents
- `/graphify` becomes `/compass`
- `graphify-out/` becomes `compass-out/`
- Graphify-specific package installation and Python interpreter discovery are removed
- References to Graphify-only commands, modules, environment variables, and providers are removed or rewritten
- Platform-specific trigger wording and progressive-disclosure references remain when they apply to Compass

The skill must not tell an assistant to install a Python package, run a Python
module, locate a Python interpreter, or configure a non-Compass service.

## Generated integrations

Always-on files, registration sections, hooks, plugin files, commands, and workflow files use the `compass` owner name. Hook commands resolve the active Compass executable and invoke native Compass subcommands.

Generated headings use `## compass`. Generated plugin filenames use `compass.js`. Generated workflow and rule filenames use `compass.md`. Kilo and similar command surfaces register `/compass`.

Uninstall removes these Compass-owned files and sections. It leaves files and sections owned by Graphify unchanged, even when both products are installed.

## Data flow

The user selects a platform through `compass install`, its platform option, or a direct platform command. The installer resolves the destination, loads the embedded Compass assets, writes the skill and references atomically, writes the version marker, then registers any platform-specific always-on integration.

Reinstall replaces Compass-owned content with the version bundled in the current binary. If a destination contains user-owned content without a Compass ownership marker, the installer returns an actionable error instead of overwriting it.

## Errors and safety

Unknown platforms and invalid option combinations fail before the installer writes files. Missing embedded assets fail with a Compass-specific reinstall message. Partial platform registration failures return a nonzero exit status and identify the affected path.

The installer confines deletion to resolved Compass-owned files. It does not delete Graphify directories, Graphify registration sections, or `graphify-out/`.

## Testing

Native Rust contract tests replace Python Graphify parity tests for this feature. Tests run the `compass` binary with isolated project and home directories.

The test suite verifies:

- Every supported platform installs into a `compass` destination
- Installed frontmatter declares `name: compass`
- Installed artifacts use native Compass commands and `compass-out/`
- Installed artifacts contain no stale `graphify`, `graphifyy`, `GRAPHIFY_*`, or `graphify-out` references
- The core index and embedded reference bundle have exact path coverage
- The native build-time skill guard rejects undersized, unlinked, or non-native assets
- Direct and generic install commands create equivalent artifacts
- Reinstall is idempotent
- Uninstall removes Compass artifacts and preserves adjacent Graphify fixtures
- `--purge` removes `compass-out/` without removing `graphify-out/`
- Parser errors do not mutate the filesystem

Focused installer tests run before the full Compass workspace tests. The repository’s Graphify knowledge graph is refreshed after code changes.
