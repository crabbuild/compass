# Proposal: preflight canonical graph size estimation

## Why

An oversized repository can spend several minutes extracting a graph only to
discover at publication that its canonical payload exceeds the fixed 2 GiB
snapshot limit. Discovery metadata is already available before extraction and
can provide a deterministic, bounded early estimate.

## What Changes

- Estimate canonical graph bytes immediately after source discovery.
- Calibrate the estimate from a measured deterministic fixture.
- Return the existing typed, actionable snapshot limit error when the estimate
  exceeds the publication bound.
- Treat files above the parser admission limit as inventory-only records.

## Compatibility

This adds an earlier failure for inputs estimated to exceed the existing
publication limit. It does not change the limit, graph schema, successful
output, or recovery controls.
