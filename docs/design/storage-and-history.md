# Storage and history design

Compass has two storage models: a mutable current-tree artifact directory and
an immutable exact-revision history store. They solve different problems and
have different recovery rules.

> **Who this page is for:** history contributors, maintainers operating large
> repositories, and integrators requiring reproducible graphs.
>
> **You will learn:** current artifact publication, incremental caches,
> historical fingerprints, Prolly trees, SQLite durability, jobs/leases,
> validation, export, and garbage collection.
>
> **Prerequisites:** [Versioned graph history](../guides/versioned-history.md).
>
> **Reading time:** 12–15 minutes.

## Two storage planes

```text
Current working tree                    Exact Git history
--------------------                    -----------------
compass-out/                            <git-common-dir>/compass/
mutable artifact set                   immutable realizations
fast incremental rebuild               content-addressed Prolly trees
can include uncommitted files          exact commit only
one practical current result           multiple profiles per commit
```

Do not use one as if it were the other.

## Current-tree artifacts

The normal pipeline writes:

```text
compass-out/
├── graph.json
├── GRAPH_REPORT.md
├── graph.html
├── manifest.json
└── cache/ and optional sidecars
```

### Manifest and cache

The manifest records the inputs needed to identify unchanged, changed, added,
renamed, or deleted work. Cached per-file extraction can be reused when its
fingerprints and parser/extractor inputs remain compatible.

Query loading may build binary caches beside `graph.json`, keyed by graph file
signature. These caches accelerate repeated reads and are disposable; the JSON
graph is authoritative.

### Publication

Atomic filesystem helpers write temporary data and replace completed outputs.
Build guards distinguish a successful complete build from interrupted state.

Consumers should use the producing command's successful exit as the handoff
point, not poll for the first appearance of a file.

## Historical identity

A history lookup begins with:

```text
Git revision -> exact commit ID
```

The storage identity also includes an extraction fingerprint:

```text
fingerprint inputs:
  build profile
  graph and canonical-encoding versions
  parser/extractor/analyzer versions
  provider and model configuration
  meaning-affecting exclusions/options
```

Excluded:

- credentials;
- machine-local paths;
- timings;
- token counts;
- operational metadata that does not change graph meaning.

This lets the same commit have multiple immutable realizations without
silently mixing them.

## Artifact partitioning

A complete realization includes more than nodes and edges:

- graph document;
- hyperedges;
- communities and analysis;
- semantic/inferred data;
- reconstruction metadata;
- authoritative sidecars;
- artifact registry entries and renderer versions;
- completion evidence.

Large maps are partitioned into typed Prolly trees. Canonical keys encode
record class and identity so diffs and reconstruction remain deterministic.

## Prolly trees

A Prolly tree is a content-addressed ordered map whose chunk boundaries are
determined by content. Similar versions can share unchanged nodes:

```text
version A root ----> branch ----> leaf 1
                           \----> leaf 2

version B root ----> branch' ---> leaf 1   shared
                            \---> leaf 2'  changed
```

This supports:

- structural sharing across nearby commits;
- deterministic roots;
- efficient key-range comparison;
- reconstruction without storing one full duplicate JSON file per revision.

The public contract is the validated realization and export, not the internal
shape of one SQLite table.

## SQLite durability

`prolly-store-sqlite` is pinned and configured with:

- write-ahead logging;
- full synchronous durability;
- a busy timeout.

The live resource set includes:

```text
history.sqlite
history.sqlite-wal / shared-memory state as applicable
jobs
leases
locks
protected worktree records
other operational files
```

Copying only `history.sqlite` while writers are active can omit committed WAL
state. Backup and restore must treat the SQLite resource coherently.

## Publication protocol

Historical publication is staged:

```text
build complete artifacts
       |
       v
validate sizes, keys, records, references, evidence
       |
       v
prepare content-addressed trees and registry
       |
       v
atomic publication transaction
       |
       v
optional preferred-pointer update
```

An incomplete, invalid, or provider-failed candidate cannot become preferred.
Published realization content is immutable.

## Preferred realizations

Preference gives read commands a default when a commit has multiple valid
profiles.

Rules:

- preference is stored separately from immutable content;
- selection validates the target realization;
- an unreadable preferred pointer is not overwritten silently;
- corrupt replacement requires explicit `rebuild --replace-corrupt`;
- replacement uses compare-and-swap observation to avoid racing another
  repair.

## Jobs and leases

Eager hooks enqueue durable work and return quickly. A worker:

1. reads FIFO jobs;
2. claims or joins through a lease;
3. heartbeats while materializing;
4. publishes or records a terminal diagnostic;
5. continues to later jobs even after one failure.

Leases prevent duplicate workers from casually publishing the same work while
allowing recovery from expired ownership.

Operational job state is not embedded into immutable graph values; it can age
out independently.

## Historical checkout isolation

Materialization uses a protected worktree under a restrictive policy:

- exact detached commit;
- offline;
- no fetch or credential prompt;
- no user hooks;
- no recursive submodules;
- no LFS smudge;
- no external-code checkout filters;
- committed ignore policy only.

This is both reproducibility and security design. A historical query should not
execute arbitrary repository-controlled checkout behavior.

## Diff model

Diff compares typed records:

- nodes;
- edges;
- hyperedges;
- optional locations;
- optional analysis;
- optional metadata.

Topology-only diff avoids reconstructing irrelevant payloads and is qualified
against the full diff.

Normal comparison checks fingerprint compatibility first. An explicit mismatch
flag permits inspection, not semantic equivalence.

## Export and reconstruction

`graph-json` reconstructs canonical graph JSON.

`compass-out`:

- restores authoritative sidecars exactly;
- regenerates derived report/HTML only with recorded renderer versions;
- validates realization completeness before reconstruction.

Meaning is judged canonically. JSON object ordering can vary; relationship
multiplicity and authoritative bytes cannot.

## Garbage collection

Default GC preserves all published realizations and removes:

- unreachable content-addressed nodes;
- expired operational records.

Non-preferred realization pruning is separate, dry-run by default, and requires
`--yes` to apply.

Logical reclamation does not mean the SQLite file immediately shrinks. GC does
not promise `VACUUM`.

## Recovery principles

```text
Disposable current cache corrupt?
  -> remove/rebuild the disposable cache, retain graph JSON

Current artifact set incomplete?
  -> rerun update to a known output, inspect build guard/diagnostic

Historical realization invalid?
  -> list/show/validate, rebuild explicitly

Preferred pointer corrupt?
  -> use replace-corrupt workflow, not manual SQLite editing

Live database backup inconsistent?
  -> restore coherent SQLite/WAL backup

Lease appears stuck?
  -> inspect job/lease state and expiry; do not delete live files blindly
```

## Related pages

- [Versioned graph history](../guides/versioned-history.md)
- [History crate tour](../implementation/workspace-tour.md)
- [Output reference](../reference/outputs.md)
- [Security and privacy](security-and-privacy.md)

**Next step:** inspect `compass history list HEAD --format json` in a disposable
repository and identify commit, realization, fingerprint, and preference.
