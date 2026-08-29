# Refinement decisions — C-019

- Qualify only exported/installed package content, never the source template
  that generated it.
- Pin exact harness versions in the canonical inventory and fail explicitly on
  absence or mismatch rather than silently weakening the lifecycle matrix.
- Use isolated temporary configuration roots, credential-free local MCP
  fixtures, bounded output capture, and deterministic redaction.
- Treat user-authored instructions and unrelated plugins as immutable sentinels
  through installation, upgrade, and uninstall.
- Keep lifecycle and OpenCode integration as phase-end CI gates after production
  implementation, consistent with the immutable phase-first doctrine.
