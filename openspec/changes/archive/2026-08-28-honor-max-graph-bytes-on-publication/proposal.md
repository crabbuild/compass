# Proposal: honor graph-byte override on publication

## Why

Compass graph readers honor `COMPASS_MAX_GRAPH_BYTES`, but snapshot preflight,
publication, validation, and streaming retained a fixed 2 GiB bound. This made
the documented opt-in ineffective on the canonical publication path.

## What Changes

- Centralize the effective snapshot byte limit in `compass-store`.
- Apply it to preflight, canonical digesting, manifest validation, snapshot
  streaming/full reads, legacy publication, and delta eligibility.
- Advertise the now-shipped override in the actionable limit error.

## Compatibility

The default remains 2 GiB. The override is explicit and process-local. Invalid,
zero, overflowing, or platform-unrepresentable values fall back to the default.
