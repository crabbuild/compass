# Refinement decisions — C-015

- Preserve the shared engine-neutral structural query contract and the optional
  feature-isolated Surreal implementation as experimental evidence.
- Replace graph-sized Surreal transactions with incomplete-manifest claims and
  resumable 512-record autocommit batches; readers switch only through the final
  repository pointer and never observe incomplete candidates.
- Treat zero semantic mismatches as necessary but not sufficient for adoption.
- Apply the C-013 thresholds exactly as ratified before Surreal results existed.
  The query-regression and native-value failures invoke the recorded falsifier.
- Do not add a CLI/MCP route, claim performance value, weaken thresholds, or
  archive the OpenSpec change as accepted behavior after the `REJECT` result.
- Stop cost-heavy release-artifact and exact-medium recovery measurements once
  decisive numeric failures make adoption impossible; record them as not
  completed rather than implying a pass.
