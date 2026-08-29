## Why

Future storage and graph-engine comparisons need one frozen, deterministic
qualification surface. Without versioned corpora, scale generators, current-engine
baselines, and precommitted budgets, later performance and quality conclusions can
be tuned to whichever implementation has already been measured.

## What Changes

- Ratify the research document's numeric qualification budgets unchanged and
  preserve their pre-measurement provenance.
- Add deterministic `qualification-medium` and `qualification-large` graph
  generators plus bounded validation and traversal tools.
- Version a semantic corpus, a 30-task agent suite, umbrella invocations, and
  focused-skill boundary prompts.
- Record reproducible current-engine binary, cold-start, query p95, and peak-RSS
  baselines before Wave 5 measurements.

## Capabilities

### New Capabilities

None. These are developer qualification artifacts, not a shipped command or
runtime dependency; `.openspec.yaml` opts out of delta specs.

### Modified Capabilities

None.

## Impact

Artifacts live under `benchmarks/qualification/` and `docs/future/`. The baseline
runner keeps generated medium graphs and raw run logs disposable under `target/`;
standalone generator and raw-traversal outputs may also use `/tmp`. No default
product dependency, public command, or compatibility contract changes.
