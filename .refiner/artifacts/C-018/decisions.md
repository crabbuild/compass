# Refinement decisions — C-018

- Make `distribution.toml` the only versioned source for supported harnesses,
  package metadata, paths, skills, and exact tool versions.
- Generate every host-native package from embedded checked inputs; do not rely
  on a source checkout at install or validation time.
- Preserve existing `compass install` and generic Agent Skills compatibility;
  the new native package export is additive.
- Keep the OpenCode bridge thin and delegate all graph behavior to the native
  Compass MCP server.
- Publish package trees atomically and validate a closed digest inventory before
  a generated package is accepted.
