# Refinement decisions — C-010

- Kept MCP 2026-07-28 as the normative server contract because official current
  documentation identifies it as the latest published revision.
- Treated independent stdio and HTTP conformance as the merge gate.
- Preserved the exact named-client matrix as release evidence, including a real
  tool invocation for every passing cell.
- Recorded OpenCode 1.18.25 HTTP as incompatible because it requests the older
  2025-11-25 initialize/GET lifecycle; did not relabel it as passing.
- Applied the user's explicit direction to use the latest MCP revision, so a
  lagging client does not force Compass to reintroduce removed behavior.
- Kept regressions in current conformance and previously passing client cells
  blocking, distinguishing server defects from external version lag.
