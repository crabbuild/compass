# Compass skill generator guard

Compass validates its installed assistant assets at build time.

The canonical native skill lives under
`crates/compass-cli/assets/compass-skill/`. `compass-cli/build.rs` loads this
module before embedding assets and fails the build when:

- the skill frontmatter is not named `compass`;
- the Codex UI metadata is missing or pre-approves tools;
- required core workflow sections disappear;
- the core exceeds a conservative token budget, or the core, any reference, or
  the complete bundle is unexpectedly small;
- a bundled reference is not linked exactly once from the core index;
- the core links a reference that is not bundled;
- the public rich-help catalog and CLI dispatch disagree;
- any public command lacks installed skill guidance;
- internal worker commands lose their explicit do-not-invoke boundary;
- the exact platform-integration asset set drifts;
- an always-on integration loses graph, query, update, or source-verification
  guidance;
- a platform command stops delegating to the canonical installed skill;
- an installed asset contains a retired product name or a Python module command;
- a reference has no level-one heading or Compass command/path.

Input files are sorted before embedding, so the generated Rust asset table is
deterministic. The optional `agents/openai.yaml` file is validated and installed
alongside the portable bundle for Codex-aware UI metadata; it must not grant
tool permissions. Adding a reference requires adding the file and its
`references/<name>.md` entry to `SKILL.md`; removing one requires removing both.
Adding a public CLI command requires adding a public page to `src/help.rs`, a
dispatch arm, and a real workflow or boundary in the skill bundle. The build
compares the rich-help catalog directly with `src/lib.rs`, so CLI, help, and
skill coverage cannot drift independently.

Run the normal focused build or test command to execute the guard:

```bash
cargo test -p compass-cli --lib install_commands
```
