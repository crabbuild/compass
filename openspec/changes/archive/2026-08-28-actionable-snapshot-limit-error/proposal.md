# Proposal: actionable snapshot publication limit error

## Why

When canonical graph publication exceeds the fixed 2 GiB snapshot limit,
Compass currently reports the bound but gives no recovery action after an
expensive build. Existing scoping controls already provide the safe remedy.

## What Changes

- Classify an oversized canonical graph as a snapshot limit failure.
- Name `--exclude <pattern>` and `.compassignore` as concrete recovery paths.
- Preserve the default limit and do not advertise `COMPASS_MAX_GRAPH_BYTES`
  until C-004 makes that override effective on this publication path.
- Add a binary-level CLI regression for rendered stderr and exit status.

## Compatibility

This intentionally extends public human-readable error text and corrects the
error category from corruption to a resource-limit failure. No command syntax,
serialized format, success output, or exit-code convention changes.
