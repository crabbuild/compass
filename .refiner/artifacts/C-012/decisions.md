# Refinement decisions — C-012

- Kept the runner outside the workspace and deleted it after measurement.
- Required both persistent engines; in-memory behavior is not acceptance
  evidence.
- Compared one canonical semantic summary instead of engine-native file bytes.
- Tested application-level immutable generation activation: partial candidates
  may remain unreachable, but cannot become active after a killed writer.
- Stored confidence as a six-decimal string to make round-trip equality exact.
- Recorded resource values without budget verdicts because C-013 must ratify
  budgets independently and before later Surreal measurements are considered.
- Kept license capture and release approval distinct: exact bytes are evidence,
  while C-011 retains decision authority.
