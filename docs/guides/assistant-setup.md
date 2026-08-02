# Set up Compass for a coding assistant

Compass embeds assistant integration assets in its native executable. This
guide explains automatic detection, explicit multi-agent selection, scope,
verification, and safe removal.

> **Who this guide is for:** developers using Compass with coding assistants
> and maintainers deciding what agent instructions belong in a repository.
>
> **You will learn:** user versus project scope, automatic and explicit platform
> selection, strict mode, verification, upgrades, and safe uninstall.
>
> **Prerequisites:** the `compass` executable installed.
>
> **Completion time:** about 2 minutes, plus the first graph build.

## What the integration does

When a graph exists, the installed skill teaches an assistant to use it before
reading a large set of raw files:

```text
architecture question
      |
      v
read compass-out/GRAPH_REPORT.md for broad context
      |
      v
run compass query for a focused subgraph
      |
      v
open only the source files needed to verify the answer
```

It does not give Compass permission to run arbitrary external actions. It
provides task instructions and, where the platform supports them, helper
integration files. Installation does not build `compass-out/`; the report
offers the exact build and activation actions when a project graph is absent.
For a repository-wide architecture, dependency, history, or impact question,
the installed guidance lets the assistant run the local deterministic first
build and continue without another setup exchange.

## Recommended setup

```bash
compass install
```

Inside Git, Compass resolves the repository root, detects supported agents, and
always includes the portable Agent Skills target. Outside Git, it uses user
scope. The report lists detection evidence, every destination, and reload or
graph-build actions.

That is the normal setup path. You do not need to identify an agent, copy a
skill directory, edit an instruction file, or build a graph before running it.
Inside a repository this command creates project-scoped, reviewable files; use
`compass install --user` instead when you want personal configuration only.

Confirm that your intended host, such as `codex` or `gemini`, appears under
`Selected` in the report. If the report selects only `agents`, the portable
skill was installed but no host-specific adapter was detected. Rerun with an
explicit target, for example `compass install --platform codex` or
`compass install --platform gemini`.

## Global or project scope

### User installation

```bash
compass install --user --platform codex
```

Use user scope when:

- this is your personal tool configuration;
- many repositories should use the same skill;
- you do not want generated assistant files committed per repository.

Some rule-only adapters, including Cursor, are project-scoped because Compass
does not invent an undocumented user-level destination. The installer fails
explicitly when a selected platform has no target for the requested scope.

### Project installation

```bash
compass install --project --platform codex
```

Use project scope when:

- the team should review and share the instructions;
- the repository already defines assistant behavior;
- a CI or reproducible development environment needs explicit setup.

Project scope writes at the Git repository root even when invoked from a
subdirectory. Review `git status` before committing generated files:

```bash
git status --short
git diff -- . ':!compass-out'
```

Never overwrite an existing project instruction file without reviewing the
merged result.

## Select one or more platforms explicitly

Run:

```bash
compass install --help
```

The current native installer recognizes the platforms printed by that command,
including Codex, Claude-family layouts, Agent Skills layouts, Gemini, Cursor,
and other supported assistants.

Examples:

```bash
compass install --platform codex
compass install --platform codex --platform claude
compass install --platform agents --platform gemini
compass install --platform cursor
compass install --all --dry-run
```

`skills` is accepted as an alias for the generic `agents` target. Exact
destinations differ by platform and scope; use installer output as the source
of truth instead of copying paths from another tool.

Detection accepts valid host configuration files, existing host directories,
and platform-specific executables. A generic editor executable alone is not
treated as proof that its AI assistant is installed. Use `--platform` when you
want a target that is not detected automatically.

Explicit platform selection bypasses detection. `--all` selects every registry
entry and conflicts with `--platform`. For CI, add `--require-all` and
`--format json` when skipped or failed targets must fail the job.

## Understand project destinations

Representative project-scoped destinations include:

```text
Codex       .agents/skills/compass/SKILL.md + AGENTS.md + .codex/hooks.json
Gemini      .agents/skills/compass/SKILL.md + GEMINI.md + .gemini/settings.json
OpenCode    .agents/skills/compass/SKILL.md
Copilot     .agents/skills/compass/SKILL.md
Agents      .agents/skills/compass/SKILL.md
Claude      .claude/skills/compass/SKILL.md + CLAUDE.md + .claude/settings.json
Kiro        .kiro/skills/compass/SKILL.md
Cline       .cline/skills/compass/SKILL.md
Cursor      Cursor-specific project integration
```

The shared consumers above write one package, not five copies. Its ownership
manifest records every consumer and content digest. The installer can also
write companion integration files required by a selected platform. Treat the
printed file list and `git status` as the authoritative result.

## Strict mode

For a supported Claude Code project installation:

```bash
compass install --project --platform claude --strict
```

Strict mode blocks the first raw file read in a session until one
`compass query` runs. It is designed to enforce a graph-first start without
trapping the entire session.

Runtime control:

```bash
COMPASS_HOOK_STRICT=0 your-assistant-command
```

Strict mode:

- requires project scope;
- currently requires the Claude platform;
- is not a security sandbox;
- should be explained to repository contributors before adoption.

## Verify the installation

### Inspect files

```bash
git status --short
```

For a project install, open the installed `SKILL.md` and any referenced files.
Check:

- commands use `compass`, not a stale product name;
- output paths use `compass-out/`;
- repository instructions are preserved;
- no machine-specific path or credential was written;
- platform-specific syntax matches the selected assistant.

For Codex project installs, open `/hooks` in Codex and review the exact Compass
hook before trusting it. Codex skips new or changed project command hooks until
they are trusted. The Compass hook reads bounded tool metadata and adds a short
graph-first reminder before matching raw searches; it does not execute a graph
build or change the proposed command. Hook trust activates the hook only; start
a new Codex session to ensure the newly installed skill and `AGENTS.md` guidance
are discovered.

For Gemini CLI, run `/skills reload` to activate a new skill in the current
session. For other hosts, start a new session or use the host's skill reload
operation when it provides one. The install report prints these actions.

### Exercise first use

In a project without a graph:

1. ask the assistant a broad architecture question;
2. confirm the assistant—not the installer—runs one local `compass update .`;
3. confirm it runs a focused query and uses `compass-out/GRAPH_REPORT.md` for
   repository-wide context;
4. confirm it verifies graph results in source.

When a graph already exists, the assistant should query it directly, open only
the source needed to verify the result, and avoid treating inferred or ambiguous
edges as unquestionable runtime truth.

### Verify idempotence

Run the same install command again:

```bash
compass install --project --platform codex
```

The result should update managed content without duplicating sections
indefinitely. Review the diff.

## Upgrade

After upgrading the Compass binary, rerun the same installation command:

```bash
compass install --project --platform codex
```

This refreshes embedded assets for that version. Review changes like any
dependency or generated configuration update.

For user installs, record the Compass version in workstation/bootstrap
automation if reproducibility matters.

## Uninstall

Use the native lifecycle command:

```bash
compass uninstall --project --platform codex
```

For user scope, select `--user` explicitly:

```bash
compass uninstall --user --platform codex
```

`--purge` is a stronger removal mode:

```bash
compass uninstall --project --platform codex --purge
```

Before using `--purge`, inspect `compass uninstall --help` and the target
files. Managed-section removal and full-file deletion have different recovery
implications.

After uninstall:

```bash
git status --short
```

Confirm that user-authored instructions remain. Restore only from version
control or backup when you are certain a removed file was meant to be tracked.

## Repository instructions

A healthy repository-level instruction is small and verifiable:

```text
When compass-out/graph.json exists, use it as the first navigation layer.

Before answering architecture questions:
1. read compass-out/GRAPH_REPORT.md;
2. run compass query for the focused question;
3. verify graph claims in source.

After modifying source files, run compass update . unless the user prohibited
generated files.
```

Avoid:

- demanding that every trivial task rebuild the graph;
- claiming the graph replaces source verification;
- giving the assistant broad external-write authority;
- checking secrets or machine-local paths into agent instructions;
- duplicating a large generated skill in several instruction files.

## Troubleshooting

| Problem | Action |
| --- | --- |
| Unknown platform | Use a name printed by `compass install --help` |
| Project files appear in an unexpected place | Compass installs at the Git root; confirm the repository and explicit scope |
| Existing instructions changed | Inspect the diff; uninstall managed content and reapply after resolving ownership |
| Strict mode blocks unexpectedly | Set `COMPASS_HOOK_STRICT=0` for the session, then review the project hook |
| Assistant ignores the graph | Confirm it discovers the installed skill and that `compass-out/` exists |
| Codex hook is skipped | Open `/hooks`, review the project hook source, and trust its current definition |
| Gemini does not see a new skill | Run `/skills reload` or start a new Gemini CLI session |
| Assistant over-trusts the graph | Strengthen instructions to verify source and qualify provenance |
| Upgrade leaves stale content | Rerun install with the same scope/platform and review managed files |

## Related pages

- [Getting started](../getting-started.md)
- [Explore a codebase](exploring-a-codebase.md)
- [Security and privacy](../design/security-and-privacy.md)
- [Troubleshooting cookbook](../cookbook/troubleshooting.md)

**Next step:** ask the configured assistant one architecture question and
verify that it narrows through Compass before opening source files.
