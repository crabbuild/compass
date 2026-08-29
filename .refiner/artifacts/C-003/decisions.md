# C-003 refinement decisions

- The current 16 GB external checkout is not a valid size-isolation corpus: it
  fails with a worker stack overflow after 73.82 s. Use the handoff-approved
  deterministic synthetic equivalent.
- Calibrate at the measured 60.62x source-to-canonical ratio and round down to
  60x instead of adding an unmeasured safety factor.
- Account for parser-rejected sources as fixed inventory records so a sparse or
  generated oversized file does not receive a false extraction multiplier.
- Run preflight after verified-output reuse checks but before any cold
  extraction, preserving fast warm builds.
