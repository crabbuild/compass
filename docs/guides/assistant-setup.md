# Set up Compass for a coding assistant

Compass embeds assistant integration assets in its native executable. This
guide explains how to select a platform and scope, inspect what was installed,
and remove it safely.

> **Who this guide is for:** developers using Compass with coding assistants
> and maintainers deciding what agent instructions belong in a repository.
>
> **You will learn:** global versus project scope, platform selection, strict
> mode, verification, upgrades, and safe uninstall.
>
> **Prerequisites:** the `compass` executable installed.
>
> **Completion time:** 5–10 minutes.

## What the integration does

The installed skill teaches an assistant to use the graph before reading a
large set of raw files:

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
integration files.

## Global or project scope

### Global installation

```bash
compass install --platform codex
```

Use global scope when:

- this is your personal tool configuration;
- many repositories should use the same skill;
- you do not want generated assistant files committed per repository.

### Project installation

```bash
compass install --project --platform codex
```

Use project scope when:

- the team should review and share the instructions;
- the repository already defines assistant behavior;
- a CI or reproducible development environment needs explicit setup.

Project scope writes under the current project. Review `git status` before
committing generated files:

```bash
git status --short
git diff -- . ':!compass-out'
```

Never overwrite an existing project instruction file without reviewing the
merged result.

## Select a platform explicitly

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
compass install --platform agents
compass install --platform gemini
compass install --platform cursor
```

`skills` is accepted as an alias for the generic `agents` target. Exact
destinations differ by platform and scope; use installer output as the source
of truth instead of copying paths from another tool.

If no platform is passed, the installer uses its documented/default detection
behavior. Teams should prefer an explicit platform in setup scripts.

## Understand project destinations

Representative project-scoped destinations include:

```text
Codex       .codex/skills/compass/SKILL.md
Agents      .agents/skills/compass/SKILL.md
Claude      .claude/skills/compass/SKILL.md
Gemini      Gemini-specific skill/config files and GEMINI.md integration
Cursor      Cursor-specific project integration
```

The installer can also write companion reference or integration files required
by the selected platform. Treat the printed file list and `git status` as the
authoritative result.

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
- applies only where the platform's hook mechanism supports it;
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

### Exercise the workflow

In a project with a graph:

1. ask the assistant a broad architecture question;
2. confirm it reads `compass-out/GRAPH_REPORT.md` or runs a focused query;
3. ask a narrow implementation question;
4. confirm it verifies graph results in source;
5. check that it does not treat inferred/ambiguous edges as unquestionable
   runtime truth.

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

For global installs, record the Compass version in workstation/bootstrap
automation if reproducibility matters.

## Uninstall

Use the native lifecycle command:

```bash
compass uninstall --project --platform codex
```

For global scope, omit `--project`:

```bash
compass uninstall --platform codex
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
This project has a Compass graph at compass-out/.

Before answering architecture questions:
1. read compass-out/GRAPH_REPORT.md;
2. run compass query for the focused question;
3. verify graph claims in source.

After modifying source files, run compass update .
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
| Project files appear in an unexpected place | Confirm current directory and whether `--project` was passed |
| Existing instructions changed | Inspect the diff; uninstall managed content and reapply after resolving ownership |
| Strict mode blocks unexpectedly | Set `COMPASS_HOOK_STRICT=0` for the session, then review the project hook |
| Assistant ignores the graph | Confirm it discovers the installed skill and that `compass-out/` exists |
| Assistant over-trusts the graph | Strengthen instructions to verify source and qualify provenance |
| Upgrade leaves stale content | Rerun install with the same scope/platform and review managed files |

## Related pages

- [Getting started](../getting-started.md)
- [Explore a codebase](exploring-a-codebase.md)
- [Security and privacy](../design/security-and-privacy.md)
- [Troubleshooting cookbook](../cookbook/troubleshooting.md)

**Next step:** ask the configured assistant one architecture question and
verify that it narrows through Compass before opening source files.
