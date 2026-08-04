# Compass Store operations

This guide describes the local `compass-store` release contract and the
operational procedures that keep a published graph coherent. It is the
runbook for `compass-out` owners, package maintainers, and future service
adapter authors.

## Release boundary

Compass addresses every value as:

```text
(namespace, partition, key) -> opaque value
```

The namespace is the first isolation boundary. A partition is the required
locality/sharding boundary inside that namespace, and ordered scans never cross
it. The graph realization uses the namespace `compass.current.graph.v1` and
keeps catalog, immutable objects, and the active selector in separate
partitions. The contract is deliberately smaller than SQL, redb, PostgreSQL,
or DynamoDB so the same behavior can be implemented by all of them.

The first local release declares these logical, machine-readable formats:

| Format | Identifier | Role | Upgrade policy |
| --- | --- | --- | --- |
| Graph document | `compass.graph/1` | Canonical `graph.json`, interchange, inspection, and recovery | Permanent compatible engine; unknown majors fail |
| Store contract | `compass.store/1` | Namespace/partition/key values, limits, conditional writes, scans, and typed errors | Major changes require a new adapter contract |
| Graph snapshot | `compass.store.graph-snapshot/1` | Immutable manifests, content digests, ordered index roots, and selector state | Same-major patch upgrades only in the first window |
| Store reference | `compass.store.ref/1` | Binds `graph.json`, store snapshot, adapter, and generation digests | Validate before a store query |
| Backup bundle | `compass.store.backup/1` | `manifest.json`, graph, reference, and a validated adapter copy | Restore only into a new directory |

The SQLite table layout, redb file layout, WAL/SHM files, object-key spelling,
and query-index cache are implementation details. They must not be queried or
edited directly. A physical-format change is a hard cut: remove the sidecar,
run the rebuild procedure, and retain the unchanged `graph.json`.

## Current backend matrix

| Backend | Local CLI | Credentials/network | Platforms | Status |
| --- | --- | --- | --- | --- |
| SQLite | Default sidecar `compass-store.sqlite3` (use `--store json` to opt out) | None; local file only | macOS, Linux, Windows | Released local adapter |
| redb | Explicit Rust adapter `compass-store-redb` | None; local file only | CI-supported native platforms | Library/conformance adapter; not selected by the CLI |
| PostgreSQL | No released CLI adapter | Would require an explicit endpoint, credentials, TLS, and bounded client | Future service profile | Deferred |
| DynamoDB | No released CLI adapter | Would require an explicit AWS boundary, credentials, TLS, retries, and quotas | Future service profile | Deferred |

No cloud SDK, endpoint, credential, or TLS setting is read by the local store
path. A future remote adapter must expose those concerns outside the common
contract and pass the same conformance and differential suites.

## Files and durability

For an output root `DIR` the published set is:

```text
DIR/.compass-active-generation # BuildGuard publication pointer, when used
DIR/.compass-generations/<generation>/graph.json
DIR/.compass-generations/<generation>/store.ref  # with default storage or --store sqlite
DIR/.compass-store/compass-store.sqlite3          # shared by store generations
```

`graph.json` is the portable authority. When SQLite is selected (the default),
the canonical JSON artifact and a small digest-bound `store.ref` are published
as one generation. The SQLite database has one stable location outside the
generation directories; it is never copied into each generation. Immutable
objects are prepared in bounded transactions, the WAL is checkpointed, and
only then can the BuildGuard pointer select the generation. Readers validate
`store.ref`, open its immutable manifest, and remain pinned even if a later
generation is published. A failed or interrupted update leaves the previous
coherent generation selected. The JSON query index beneath the requested cache
root is disposable and may be deleted at any time.

New stores contain projected immutable graph indexes and their manifest; they
do not duplicate the complete graph as a legacy chunked payload. `graph.json`
remains complete, portable, and independently queryable. The projected roots
cover metadata/files, nodes, edges, names, terms, outgoing and incoming
adjacency, communities, and diagnostics. Tree objects use bounded compact
MessagePack with deterministic zstd compression when it reduces size.

SQLite currently uses a `WITHOUT ROWID` binary primary key over
`(namespace, partition, key)`, 16 KiB pages, WAL, `synchronous=FULL`, incremental
auto-vacuum, bounded cache/mmap settings, and explicit transactions. These are
adapter-private choices. `EXPLAIN QUERY PLAN` and native tests verify that both
point reads and partition-range scans use the primary key; users must not add
or edit physical indexes themselves.

The store contract bounds namespace, partition, key, value, scan, and graph
sizes. Those limits are correctness limits, not hints: exceeding one produces
a typed failure and must never be reported as an empty result. Local quotas
are therefore enforced by the API and available disk space; service adapters
will add explicit tenant and request quotas later.

## Health and recovery commands

Run these commands from the Compass checkout or with an installed binary:

```bash
compass store status compass-out --format json
compass store validate compass-out --format json
```

`status` is read-only and reports graph, sidecar, selector, schema, byte size,
and digest state. `validate` fails unless the sidecar, active snapshot,
`store.ref`, and canonical JSON export all agree. A failed validation does not
modify any artifact.

To create a self-contained backup:

```bash
compass store backup compass-out --output /safe/path/compass-store-backup
```

The destination must not already exist. The command checkpoints SQLite, copies
the validated database, copies `graph.json` and `store.ref`, and writes a
digest-bound `manifest.json`. Keep the bundle on storage with equivalent or
better access controls; it contains project structure and source anchors.

Restore only into a new or empty directory:

```bash
compass store restore \
  --from /safe/path/compass-store-backup \
  --into /recovered/compass-out
compass store validate /recovered/compass-out --format json
```

Restore validates every digest and snapshot before publication. It removes an
incomplete destination on failure and never overwrites an existing output.
After restore, typed queries use the restored SQLite snapshot by default. Pass
`--engine json` to force the portable reader or `--engine store` to require and
validate the restored database.

## Rebuild and upgrade policy

The first supported upgrade window is the `0.3.x` release line for the logical
format majors above. Patch releases may reopen and validate a matching
same-major SQLite store. Unknown majors, pre-release prototypes, a missing or
invalid `store.ref`, and physical files from another adapter are not migrated
in place.

Rebuild a sidecar from the current graph without replacing the JSON artifact:

```bash
scripts/rebuild_compass_store.sh . --out compass-out --compass compass
```

The script moves existing sidecars into a timestamped rollback directory,
runs `compass update --force --store sqlite`, and restores the old sidecars if
the update fails. On success it leaves the backup for an operator to review and remove
according to local retention policy. The graph file is never deleted by the
script. If source is available, a normal
`compass update --force --store sqlite` is the preferred rebuild; if only
`graph.json` is available, use the JSON engine or
import it through a separately reviewed adapter tool.

Downgrades must preserve `graph.json` and run `compass store validate` after
the target binary starts. Do not copy a newer physical sidecar over an older
binary and do not make an invalid sidecar look healthy by editing its bytes.

## Backup, GC, and retention

The local backup procedure is a file-level snapshot after writers close or the
SQLite WAL is checkpointed. Filesystem snapshots may be used by operators, but
the bundle manifest and validation command remain the portability boundary.
Redb backups use the library adapter's read-only reopen rule; they are not
currently exposed by the CLI.

The local publisher retains the active and immediately previous complete
BuildGuard generations. After a successful store publication it marks the
manifests and tree objects selected by those retained references, deletes
unreachable entries in bounded transactions, performs bounded incremental
page reclamation, and checkpoints the WAL. Staging and active references are
always retained; malformed retained references fail maintenance rather than
guessing. Backup bundles are outside this automatic policy and remain under
operator control. A future multi-tenant service still requires namespace-
scoped leases, service quotas, and an auditable distributed GC policy.

With `--timing`, publication reports `new_objects`, `reused_objects`,
`write_transactions`, `bytes_written`, and `gc_deleted_entries`. These counters
cover the store work inside publication timing and are the preferred evidence
for diagnosing write amplification.

## Incident checklist

1. Stop concurrent writers and copy the entire output directory to protected
   storage.
2. Run `compass store status --format json`; save only the sanitized response.
3. Run `compass store validate --format json` and record the typed error.
4. If the graph is valid but the sidecar is not, use the rebuild script or
   `--engine json`; do not edit SQLite tables.
5. If both are damaged, restore a validated backup into a new directory and
   compare the graph digest before switching consumers.
6. Report Compass version, platform, schema identifiers, and bounded command
   output. Do not attach databases, credentials, private source, or raw
   provider responses unless the security process explicitly requests them.

## Qualification entry point

The release harness creates a deterministic typed graph and exercises both
embedded adapters, canonical JSON export, typed search, CompassQL, transaction
and byte counters, database size, write amplification, retention, and GC:

```bash
scripts/qualify_compass_store_release.sh
```

The script writes all raw observations beneath the selected target directory;
those files are release evidence, not repository artifacts. Re-run it on the
release commit after the native gates and packaging checks complete.
