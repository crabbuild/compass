# Decisions — C-002 refinement

## 2026-08-28 — Keep resource-limit and corruption classes distinct

An oversized, otherwise well-formed manifest is a typed `SnapshotError::Limit`; an empty byte count remains `SnapshotError::Corrupt`. This preserves machine classification instead of making callers parse remediation prose.

## 2026-08-28 — Advertise only effective controls

The remediation names `--exclude <pattern>` and `.compassignore`. It deliberately omits `COMPASS_MAX_GRAPH_BYTES` until C-004 wires that override into the publication path.

## 2026-08-28 — Exercise immutable reference selection in the CLI regression

The store engine pins `store.ref`, not the mutable active selector. The regression therefore publishes a digest-consistent oversized manifest and rewrites both the selector and its immutable reference before invoking the actual binary.

## 2026-08-28 — Make remediation valid for publication and reads

Manifest validation is shared by publishers and immutable readers. Use `retry or rebuild with a smaller scope` so the action is accurate in both contexts while retaining one typed, machine-classifiable limit error.

## 2026-08-28 — Keep storage layout in the graph owner

Expose the versioned manifest-key encoder and existing layout constants from `compass-graph`, then use the backend-neutral `Store` interface in the CLI contract test. This avoids raw SQLite coupling and makes the same regression portable to any conforming local adapter.

## 2026-08-28 — Accept deterministic validation fallback

The installed artifact-refiner package is incomplete. Persist the KBD constraint results and tooling degradation instead of claiming schema or validator-agent execution that the host cannot perform.
