# Harden Compass skill installation for coding agents

## Goal

Make one `compass install` command discover and configure the coding agents a
user actually has, while preserving explicit platform selection, protecting
user-owned configuration, and teaching every supported agent to use Compass as
the first navigation layer for codebase work.

The installer must work well for a person at a terminal, an AI coding agent, and
non-interactive CI. It must not claim success for an undiscoverable skill,
silently replace malformed configuration, or leave unrelated agent settings
damaged after install, upgrade, migration, or uninstall.

## User experience

Plain `compass install` is project-aware:

- Inside a Git repository, it installs project-scoped guidance at the repository
  root.
- Outside a Git repository, it installs user-scoped guidance.
- If supported agents are detected, it configures all detected agents.
- If none are detected, it installs the portable skill at
  `.agents/skills/compass` or `~/.agents/skills/compass` and explains how to
  verify discovery later.

Users can override every automatic decision:

```text
compass install
compass install --project
compass install --user
compass install --platform claude
compass install --platform codex --platform claude
compass install --all
compass install --dry-run
compass install --require-all
compass install --format json
```

`--project` and `--user` conflict. `--platform` is repeatable and bypasses
agent detection. `--all` selects every supported platform and conflicts with
`--platform`. The existing `--strict` option keeps its current meaning: enable
the Claude Code project hook that requires an initial Compass query.
`--require-all` is the separate automation option that returns a nonzero status
when any selected target is skipped or fails.

Direct compatibility commands such as `compass codex install` continue to
select one platform and use the same planner and executor as the generic
command.

## Architecture

The installer is split into planning and execution:

```text
request
  -> resolve scope
  -> detect or select agents
  -> expand platform adapters
  -> deduplicate shared destinations
  -> preflight every target
  -> execute per-target transactions
  -> verify installed artifacts
  -> report outcomes and next actions
```

### Agent registry

An `AgentRegistry` is the only source of platform aliases, detection evidence,
supported scopes, skill destinations, shared-destination membership, and
optional always-on integrations. A record contains:

- Stable platform identifier and accepted CLI aliases
- Support tier: shared skill, native skill, or instruction adapter
- Strong and weak detection signals
- Project and user destinations
- Optional environment overrides such as `CODEX_HOME` or
  `CLAUDE_CONFIG_DIR`
- Always-on instruction, hook, plugin, command, or steering adapter
- Documentation URL and last verification date
- Reload or restart guidance

Platform data must not be spread across parser lists and destination match
statements. Registry construction validates unique identifiers and aliases,
valid destinations, and complete documentation metadata.

### Scope resolver

`ScopeResolver` returns either `Project { root }` or `User { home }`. Explicit
scope wins. Automatic project scope uses the Git repository root, not the
current subdirectory, so root guidance is available throughout a repository.
When no Git root is found, automatic scope uses the user home.

Failure to resolve an explicitly requested scope is a parser/preflight error
and causes no writes.

### Detector

Detection is read-only and records its evidence. Strong signals are a matching
executable, a valid native configuration file, or an agent-specific environment
override. Existing native directories and instruction files are supporting
signals. A similarly named directory by itself is not enough to identify an
agent.

Explicit `--platform` selections never depend on detection. Plain installation
always includes the portable shared target, even if every detected agent uses a
native adapter.

### Install plan

`InstallPlanner` produces an immutable `InstallPlan` before any mutation. Each
target contains its consumers, resolved paths, preflight result, planned
actions, and rollback information.

Targets with the same normalized skill destination and identical package are
deduplicated. Codex, Gemini CLI, OpenCode, and GitHub Copilot therefore share
one `.agents/skills/compass` installation instead of receiving duplicate skill
copies.

### Executor and report

`InstallExecutor` applies independent per-target transactions. One target
failure does not block another unless parsing or scope resolution made the whole
request invalid. Each result is classified as:

- `installed`
- `updated`
- `current`
- `skipped`
- `failed`

`InstallReport` includes the scope, detected agents and evidence, selected
platforms, paths, statuses, reasons, rollback result, graph state, reload
guidance, and next actions. Text output is concise and stable enough for people.
JSON output is versioned and contains the same information for agents and CI.

Without `--require-all`, success means at least one selected target installed,
updated, or was already current; skips and failures remain prominent in the
report. With `--require-all`, any skip or failure produces a nonzero exit.
When no target succeeds, the command always returns nonzero.

## Platform policy

### Shared Agent Skills target

The following consumers use the portable Agent Skills target:

| Consumer | Project | User |
| --- | --- | --- |
| Codex | `.agents/skills/compass` | `~/.agents/skills/compass` |
| Gemini CLI | `.agents/skills/compass` | `~/.agents/skills/compass` |
| OpenCode | `.agents/skills/compass` | `~/.agents/skills/compass` |
| GitHub Copilot | `.agents/skills/compass` | `~/.agents/skills/compass` |
| Generic Agent Skills clients | `.agents/skills/compass` | `~/.agents/skills/compass` |

This corrects the current Codex destination, which uses
`.codex/skills/compass`. Current Codex documentation specifies repository and
user skill discovery under `.agents/skills`.

### Native Agent Skills targets

Agents with documented native roots keep dedicated copies:

| Consumer | Project | User |
| --- | --- | --- |
| Claude Code | `.claude/skills/compass` | `~/.claude/skills/compass` |
| Kiro | `.kiro/skills/compass` | `~/.kiro/skills/compass` |
| Cline | `.cline/skills/compass` | `~/.cline/skills/compass` |

### Instruction adapters

Cursor, Windsurf, and agents without a verified native skill root receive
minimal documented rule or instruction adapters. Existing Compass adapters for
Aider, Amp, Kilo, Trae, Droid, Devin, Pi, Hermes, CodeBuddy, Antigravity,
OpenClaw, and Windows variants remain supported through the registry.

An adapter must not invent Agent Skills support. Experimental support is labeled
in help and documentation. Adding a platform requires an official source,
fixtures, and lifecycle tests.

### Verified sources

The initial registry is based on documentation verified on 2026-07-24:

- Agent Skills specification:
  <https://agentskills.io/specification>
- Codex skills:
  <https://developers.openai.com/codex/concepts/customization#skills>
- Claude Code skills:
  <https://code.claude.com/docs/en/skills>
- Gemini CLI skills:
  <https://geminicli.com/docs/cli/skills/>
- OpenCode skills:
  <https://opencode.ai/docs/skills/>
- GitHub Copilot skills:
  <https://docs.github.com/en/copilot/how-tos/copilot-on-github/customize-copilot/customize-cloud-agent/add-skills>
- Kiro skills:
  <https://kiro.dev/docs/skills/>
- Cline skills:
  <https://docs.cline.bot/customization/skills>
- Cursor rules:
  <https://docs.cursor.com/context/rules-for-ai>
- Windsurf rules and `AGENTS.md`:
  <https://docs.windsurf.com/windsurf/cascade/memories>

## Skill and codebase-guidance contract

The canonical package remains an Agent Skills-compliant `compass` directory
with a focused `SKILL.md` and one-level `references/` bundle. It follows
progressive disclosure:

1. Agents initially see only `name` and a precise `description`.
2. The main workflow loads for architecture, dependency, history, change
   impact, project-artifact, and `/compass` requests.
3. Detailed command references load only for the relevant operation.

The skill uses the pattern proven by Graphify:

- Treat the knowledge graph as the first navigation layer.
- Read `compass-out/GRAPH_REPORT.md` for architecture, god nodes, and community
  structure.
- Navigate from `compass-out/wiki/index.md` when it exists.
- Run a focused `compass query` before broad source searches.
- Use `path`, `explain`, `affected`, or CompassQL for exact relationships.
- Verify important graph conclusions in cited source.
- Preserve edge direction, confidence, repository origin, graph path, and
  historical revision.
- Treat missing and inferred evidence honestly.
- Run `compass update .` after code changes when repository guidance requires
  it, and report refresh failures.

Always-on integration text stays short. It says to use the graph *when it
exists* and never claims that installation built `compass-out/`. When the graph
is absent, the install report recommends `compass update .`; installation does
not automatically scan the repository.

The skill frontmatter adds portable compatibility and version metadata allowed
by the Agent Skills specification. The core remains under the recommended
5,000-token activation budget. Platform-only behavior belongs in adapters, not
forked skill bodies.

## Ownership and safe mutation

Each managed skill directory contains a versioned ownership manifest. The
manifest records:

- Manifest schema and Compass version
- Scope and normalized root
- Platform consumers
- Managed relative paths
- Installed content digests
- Adapter identities

Reinstallation compares current content with recorded digests. An unchanged
managed installation can be upgraded. A user-modified managed file is skipped
as a conflict and is not overwritten automatically. Unowned content is never
overwritten.

The complete skill directory is staged in a uniquely named sibling directory
and atomically swapped where the platform permits. Concurrent operations use a
lock scoped to the installation root. Stale staging directories are cleaned
only when their ownership and age are verified.

JSON and TOML configuration is parsed strictly. Invalid content is preserved
byte-for-byte and produces an actionable result. Successful edits preserve
unknown fields and unrelated arrays or tables.

Managed Markdown uses explicit Compass start and end markers. Hook and plugin
entries use exact managed identities. Uninstall never uses substring matching
such as “contains `compass`” to decide ownership.

A failed target transaction restores changed configuration from the captured
preflight state and removes only newly created Compass-owned files. If rollback
also fails, the report identifies every affected path and never says the target
was successfully installed.

## Legacy migration

A current Compass-managed installation at `.codex/skills/compass` is a legacy
Codex target. Migration:

1. Preflights both the legacy and shared destinations.
2. Installs and verifies `.agents/skills/compass`.
3. Updates the ownership manifest and Codex integration.
4. Removes the legacy directory only when it is still Compass-managed and
   matches its recorded or bundled content.

User-modified or unowned legacy directories remain in place and are reported.
Graphify skills, `graphify-out/`, and Graphify instruction sections are outside
the Compass ownership boundary.

## Error handling

The installer explicitly handles:

- Unknown platforms and conflicting options
- No Git root, home directory, or required environment override
- Paths that are files when directories are required
- Read-only destinations and permission errors
- Malformed JSON or TOML
- Symlinked paths that cannot be resolved safely
- Duplicate aliases and shared destinations
- Concurrent installation or uninstall
- Interrupted staging and atomic rename failures
- User-modified managed files
- Missing or invalid embedded assets
- Partial adapter registration
- Legacy migrations that cannot safely finish

Errors identify the platform, action, path, reason, and recovery command. Secret
values and full environment dumps never appear in text or JSON output.

## Verification and testing

### Unit tests

- Registry uniqueness, aliases, documentation metadata, and destinations
- Scope resolution inside and outside Git, including explicit overrides
- Strong and weak detection evidence
- Shared-target expansion and deduplication
- Parser conflicts and repeated platform selection
- Ownership manifest serialization and digest comparison
- Exact Markdown, hook, and plugin ownership
- Text and JSON report status and exit-code rules

### Integration tests

Black-box CLI fixtures use isolated project, home, `PATH`, and configuration
trees. They cover:

- No-argument project and user installation
- No-agent portable fallback
- Multiple detected and explicitly selected agents
- `--all`, `--dry-run`, `--require-all`, and JSON output
- Every supported project and user destination
- Native and adapter equivalence through generic and direct commands
- Idempotent reinstall and uninstall
- Shared-skill deduplication
- Malformed configuration preserved byte-for-byte
- Permission, file/directory, symlink, and missing-home failures
- Concurrent installers and unique staging
- Interrupted per-target transactions and rollback
- User-modified managed and unowned content
- Legacy Codex migration
- Preservation of Graphify and unrelated agent artifacts
- Windows-style paths and environment overrides

### Skill contracts and evaluations

The build-time guard continues to validate frontmatter, native Compass branding,
reference coverage, public command coverage, and the internal-command boundary.
It additionally checks Agent Skills field constraints, the activation-token
budget, compatibility metadata, and one-level references.

Trigger fixtures cover explicit `/compass`, architecture, dependency, history,
impact, an existing `compass-out/`, unrelated coding prompts, and ambiguous
prompts. Optional real-client smoke tests verify discovery where a stable
non-interactive command exists. Otherwise the report gives the documented
reload or restart step, such as Gemini’s `/skills reload`.

Focused installer tests run first, followed by the complete Compass workspace
suite. After code changes, the repository Graphify graph is refreshed with
`graphify update .`.

## Documentation

The assistant setup guide documents:

- Automatic and explicit installation examples
- Project versus user scope
- Support tier and verified destination matrix
- Detection evidence
- Shared-destination behavior
- Reload and restart instructions
- `--strict` versus `--require-all`
- Upgrade, migration, conflict recovery, and uninstall
- Text and JSON result interpretation
- The graph-first workflow installed for agents

CLI help and the guide are generated or contract-tested against the registry so
the supported platform list cannot silently drift from implementation.

## Non-goals

This change does not:

- Build a graph as a side effect of skill installation
- Install an agent application
- Configure agent authentication or provider credentials
- Publish separate marketplace packages
- Overwrite user-modified or unowned files
- Remove Graphify installations or output
- Guarantee behavior for undocumented agent skill locations
