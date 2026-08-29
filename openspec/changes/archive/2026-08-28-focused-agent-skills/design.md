## Context

See `proposal.md` for motivation. `compass-cli` currently embeds `compass-skill/`, installs it transactionally into one platform-specific `skills/compass/` directory, and records checksums plus consumers in `.compass-install.json`. Multiple platforms can share the same destination. Build-time `skillgen` validation protects the canonical skill and its references.

The focused skills must be discoverable by normal Agent Skills clients, which requires sibling directories whose names match their `SKILL.md` frontmatter. They cannot be nested beneath the umbrella skill. Existing installations may contain user-owned sibling directories, so collection installation needs all-destination preflight and rollback.

## Goals / Non-Goals

**Goals:**

- Preserve the exact canonical umbrella asset while adding six narrow activation surfaces.
- Reuse the existing ownership manifest, deterministic embedding, staging, checksums, and consumer model for each focused tree.
- Make install, reinstall, multi-consumer uninstall, and rollback operate over the complete seven-skill collection.
- Keep focused skills portable and independently usable by any Agent Skills-compatible client.

**Non-Goals:**

- Changing the umbrella instructions, public CLI grammar, MCP protocol, or graph contracts.
- Adding client-specific plugin packaging; C-018 owns that distribution surface.
- Treating a deterministic trigger corpus as a guarantee about every model or client implementation.

## Decisions

### Install focused skills as managed siblings

Each focused asset is embedded under `compass-focused-skills/<name>/` and installed beside the platform's existing `compass/` directory. Every directory gets its own ownership manifest and version marker with the same consumer set. This satisfies client discovery and lets checksum validation remain local to one skill tree.

Embedding focused content inside the umbrella references was rejected because clients would not discover nested skills. Duplicating the umbrella reference bundle into every focused skill was rejected because the focused instructions can invoke stable Compass commands directly and do not need the large corpus.

### Generalize the existing package transaction

Installation first validates all seven destinations and snapshots all existing managed directories. It then installs each package through the existing stage/backup/rename primitive. Any package or adapter failure restores the entire skill collection and adapter snapshots. This preserves the established path-containment and unowned-file rules without introducing a second installer.

A single atomic rename of the whole `skills/` container was rejected because that container can hold unrelated user skills that Compass must not copy, replace, or own.

### Pin umbrella compatibility and validate focused structure at build time

The build validator keeps the umbrella checks and adds a fixed pre-change SHA-256 assertion plus focused inventory validation. Each focused directory must have matching lower-kebab frontmatter, a discriminative description, no absolute paths, and no unresolved relative resource links.

### Check activation boundaries with a portable corpus

A checked-in JSON corpus contains broad umbrella prompts and task-specific focused prompts. Tests score only explicit, documented activation terms and require a unique expected match for focused cases, with umbrella fallback for broad cases. This is deterministic regression evidence for descriptions and boundary design; it does not claim to model a client LLM.

## Risks / Trade-offs

- **[Risk] Seven manifests increase installed file count** → Keep focused trees minimal and deterministic; C-017 can expose inventory/doctor views without changing package ownership.
- **[Risk] A failure after one sibling rename cannot be filesystem-atomic across the whole container** → Preflight all destinations, snapshot all owned trees, stage each package, and roll the complete collection back on failure.
- **[Risk] Trigger words can overlap as descriptions evolve** → Keep a checked-in positive and umbrella-fallback corpus in the build/test gate and require unique focused matches.
- **[Risk] Modified focused trees could be lost during uninstall** → Verify each manifest before removal and preserve any unowned or checksum-divergent directory.

## Migration Plan

Existing users run the same `compass install` command. A current managed umbrella gains six additive sibling trees. Reinstall is idempotent. Uninstall removes managed focused trees when the last consumer leaves. Rolling back to an older Compass leaves focused trees in place because that older binary does not know or own them; reinstalling the newer version reconciles them safely.
