# Versioned Graph Resolution and Semantic Diff Mitigations

**Date:** 2026-07-22

**Status:** Approved

**Implementation root:** `/Users/haipingfu/graphify/compass`

**Amends:** `docs/superpowers/plans/2026-07-21-versioned-graph-prolly-tree.md`

## Purpose

This design closes correctness and usability gaps found while verifying Compass versioned graphs end to end against LevelDB. It preserves the approved SQLite-backed Prolly architecture while making lazy materialization use the intended build profile, preventing read operations from leaving unnecessary jobs, requiring comparable realizations for normal diffs, and distinguishing meaningful graph changes from source-location churn.

The verification established that raw Prolly diffs are deterministic and correct relative to exported graph records. The remaining problems are resolution policy and presentation:

- Lazy `query --at`, `path --at`, `explain --at`, export, and `diff` use default build options rather than the enabled repository profile.
- Resolution enqueues a job before checking whether a valid preferred realization already exists.
- Function-body changes are not represented directly when topology is unchanged.
- A one-line insertion can surface as many changed nodes and edges because `source_location` is treated like any other attribute.
- Analysis and metadata churn can obscure the architectural changes a person is trying to understand.

## Approved decisions

- One centralized resolver governs historical query, path, explain, export, and diff.
- Valid existing realizations are resolved and validated before any durable mutation.
- A normal diff requires matching extraction fingerprints.
- Cross-profile comparison requires explicit `--allow-profile-mismatch`.
- Missing history is materialized using a comparable stored profile when possible.
- Every realization stores its canonical non-secret build profile, not only a fingerprint.
- Code symbols record stable signature, implementation, and source hashes without changing their identity keys.
- Diff records are classified as semantic, textual, location, analysis, or metadata changes.
- Default text output shows semantic changes first and collapses location, analysis, and metadata churn into separate sections.
- `--topology-only` means literal topology: node and edge identity additions and removals only.
- JSON uses a versioned comparison envelope and retains every streamed record with its category.
- The versioned graph feature has not shipped, so the development v1 realization format may be amended without migration support.
- Compass is the canonical command surface; Graphify compatibility does not constrain this design.

## Goals

1. Make eager and lazy materialization use the same normalized profile semantics.
2. Ensure a successful historical read is operationally read-only when its realization already exists.
3. Prevent extraction-environment differences from masquerading as code-architecture differences.
4. Report implementation changes even when node and edge topology is unchanged.
5. Preserve complete exact diffs while presenting meaningful changes before positional and derived churn.
6. Retain streaming, bounded-memory behavior and Prolly subtree reuse.
7. Produce actionable, deterministic diagnostics for missing, corrupt, or incomparable history.

## Non-goals

- Inferring semantic equivalence between two different extraction fingerprints.
- Automatically replacing a corrupt preferred realization.
- Mutating preferred pointers merely to satisfy one diff invocation.
- Storing source text or secrets in realization manifests.
- Adding a second semantic-only Prolly tree in this version.
- Maintaining the current development-only JSON diff shape.

## Central history resolver

Introduce one resolver service used by every commit-aware read command. It owns repository discovery, revision resolution, store access, profile selection, job creation, waiting, validation, and reconstruction boundaries.

The resolver exposes three conceptual operations:

- `resolve_existing`: locate and validate an existing realization without creating a store, job, lease, hook, or temporary worktree.
- `resolve_or_materialize`: return a valid realization or materialize exactly one missing commit with a selected profile.
- `resolve_comparable_pair`: resolve two commits, enforce the requested fingerprint policy, and return validated realizations ready for Prolly diffing.

### Existing-realization fast path

The resolver must perform these steps in order:

1. Resolve the revision to an exact Git object ID.
2. Open an existing history store without creating operational state.
3. Look up the selected or preferred realization.
4. Validate the realization and all required roots.
5. Return immediately when it is valid.

Only after this fast path fails may the resolver create the history store, allocate a job, acquire a lease, or create an exact-tree worktree. Tests must assert that the job count, SQLite modification time, and operational directories do not change on the fast path.

A corrupt preferred realization fails closed. It is not silently skipped, replaced, or rebuilt. The diagnostic directs the user to `compass history rebuild REV --replace-corrupt`.

### Profile selection

When materialization is required, choose a profile in this order:

1. An explicit `--profile-from REV|REALIZATION` request.
2. The stored profile of the already-resolved counterpart in a comparable diff.
3. The enabled repository-wide `HistoryConfig.profile`.
4. Deterministic Compass defaults when no repository profile exists.

Explicit `compass history build REV` options continue to define their own normalized profile. Eager jobs capture the enabled profile at enqueue time; later configuration changes do not rewrite legitimate queued jobs.

The selected profile is canonicalized before the job ID is derived. Concurrent requests for the same commit and profile join the same non-terminal job.

## Comparable realization selection

`compass diff OLD NEW` first resolves both preferred realizations without writes.

| OLD | NEW | Behavior |
|---|---|---|
| Present, same fingerprint | Present, same fingerprint | Validate both and stream the diff. |
| Present | Missing | Materialize NEW with OLD's stored profile. |
| Missing | Present | Materialize OLD with NEW's stored profile. |
| Missing | Missing | Materialize both with the enabled profile, or deterministic defaults if history is not configured. |
| Present, different fingerprints | Present, different fingerprints | Fail before scanning Prolly trees. |
| Corrupt | Any | Fail closed with explicit recovery instructions. |

The normal mismatch error includes both commits, realization IDs, fingerprints, and concise remediation:

```text
error: realizations are not semantically comparable

OLD fingerprint: abc…
NEW fingerprint: def…

Build a comparable realization:
  compass history build NEW --profile-from OLD

Or inspect intentionally:
  compass diff OLD NEW --allow-profile-mismatch
```

`--fingerprint SHA` selects existing valid realizations with that fingerprint on both commits without changing preferred pointers. A missing or ambiguous selection is an error. `--profile-from` is the explicit mechanism for creating a missing comparable realization.

`--allow-profile-mismatch` bypasses only the comparability check. It never changes profile data or preferred pointers. Text mode writes a prominent warning to stderr. JSON records `profile_mismatch: true` in the comparison metadata.

## Immutable profile provenance

Each realization manifest stores:

- The normalized non-secret `BuildProfile`.
- Its profile digest.
- The extraction fingerprint derived from all meaning-affecting inputs.
- Canonicalization, definition-hash, graph-schema, extractor, resolver, and analysis versions.

Publication validates that the stored profile digest matches the canonical profile bytes and agrees with the fingerprint inputs. Profiles must not contain API keys, bearer tokens, authorization headers, environment values, or credential-bearing endpoints.

Operational job files may be cleaned without losing the ability to reproduce a realization. Reproduction depends only on immutable realization provenance and available source/tool artifacts.

## Stable symbol change evidence

Code-symbol records gain three optional versioned attributes:

- `signature_hash`: a hash of the normalized declaration or public interface.
- `implementation_hash`: a hash of the normalized implementation AST.
- `source_hash`: a hash of the exact definition slice after line-ending normalization.

These attributes do not participate in node identity. A symbol moved within or between files retains its identity under the existing canonical ID rules.

The implementation hash excludes absolute positions, whitespace, and comment nodes. Its input is a deterministic representation of node kinds, operators, identifiers, literals, and ordered named children. The algorithm version is part of the realization fingerprint. When a supported parser cannot provide a trustworthy definition boundary, Compass omits the hash and records the limitation instead of fabricating one.

The source hash detects formatting and comment-only edits that do not alter the normalized implementation. Compass stores only digests, never source slices.

## Diff classification

The existing typed Prolly diff remains the source of raw changes. A bounded classifier processes each `GraphChange` independently.

Classification first uses the named root that produced the record. Every analysis-tree record is **analysis**, and every metadata-tree record is **metadata**, including additions and removals. Records from the node, edge, and hyperedge trees then use this precedence:

1. **Semantic:** record identity added or removed; signature, implementation, topology, relation, confidence, or another meaning-bearing attribute changed.
2. **Textual:** only the exact source hash changed while normalized signature and implementation hashes remained equal.
3. **Location:** old and new records become equal after removing position, span, line, and column fields.

Community assignments, normalized labels, questions, cycles, surprises, and other analysis outputs belong to the analysis category. Artifact-registry, manifest, provenance, build-state, and other metadata outputs belong to the metadata category.

Node and edge keys remain unchanged. Location fields remain stored so historical explanation and navigation preserve exact coordinates.

## Command behavior

The command surface becomes:

```text
compass diff OLD NEW
  [--detailed]
  [--format text|json]
  [--topology-only]
  [--include-locations]
  [--include-analysis]
  [--include-metadata]
  [--fingerprint SHA]
  [--allow-profile-mismatch]

compass history build REV [build-profile options]
  [--profile-from REV|REALIZATION]
```

`--profile-from` conflicts with direct build-profile options. `--fingerprint` conflicts with `--allow-profile-mismatch` because the former requests comparability and the latter waives it.

### Text output

Default text output orders sections by usefulness:

```text
Semantic graph changes
  5 nodes added
  8 edges added
  1 implementation changed

Textual changes
  2 definitions changed without semantic AST changes

Analysis changes
  27 community assignments changed (collapsed)

Location changes
  161 records moved across 1 file (collapsed)

Metadata changes
  3 records changed (collapsed)
```

`--detailed` expands semantic and textual changes but leaves the other sections collapsed. The corresponding `--include-*` option expands a collapsed category. Examples remain bounded.

`--topology-only` emits only node and edge identity additions and removals. Attribute, implementation, textual, location, analysis, and metadata changes are excluded. A function-body-only commit therefore reports an implementation change normally and `no topology changes` under `--topology-only`.

### JSON output

JSON uses a versioned envelope:

```json
{
  "schema_version": 2,
  "comparison": {
    "old_commit": "...",
    "new_commit": "...",
    "old_realization": "...",
    "new_realization": "...",
    "old_fingerprint": "...",
    "new_fingerprint": "...",
    "profile_mismatch": false
  },
  "changes": [
    {
      "category": "semantic",
      "record": "node",
      "change": "changed",
      "key": ["..."],
      "old": {},
      "new": {}
    }
  ],
  "summary": {
    "semantic": 12,
    "textual": 2,
    "location": 161,
    "analysis": 27,
    "metadata": 3
  }
}
```

The writer emits comparison metadata, streams the `changes` array, and appends the bounded summary. A broken or short writer aborts the Prolly scan promptly. JSON ordering is deterministic for identical roots and options.

## Error and recovery behavior

- Unknown revisions fail before opening or creating history storage.
- Missing provider credentials or unsupported historical inputs fail without publishing a candidate.
- Fingerprint mismatches fail before reading changed Prolly subtrees.
- Corrupt preferred state requires explicit rebuild recovery.
- A failed materialization remains diagnostic operational state and never becomes preferred.
- A valid existing realization returns without creating or joining a job.
- An explicit mismatched comparison remains read-only and clearly marks the mismatch.
- Temporary exact-tree worktrees are removed after success, failure, cancellation, or process recovery.

## Performance and boundedness

The classifier does not materialize complete graphs. It retains category counts, bounded examples, and renderer state. Raw JSON changes continue streaming directly to the output writer.

Equal stored roots return without node reads. Different roots continue using `Prolly::stream_diff`, preserving shared-subtree skipping. Adding hashes changes only the symbol records whose definitions are affected; unchanged records and subtrees remain content-addressed and reusable.

No second semantic-view tree is introduced until measurements show classification itself is a bottleneck.

## Verification requirements

### Resolver and job lifecycle

- Enabled profile exclusions are honored by lazy diff, query, path, explain, and export.
- A valid preferred realization creates no store, job, lease, hook, or temporary worktree.
- One missing diff side inherits the other side's stored profile.
- Two missing sides use one identical enabled or default profile.
- Concurrent requests join one commit/profile job.
- Corrupt preferred state fails without automatic replacement.
- Legitimate queued jobs retain the profile captured when they were enqueued.

### Comparability

- Matching fingerprints stream normally.
- Mismatched fingerprints fail before Prolly node reads.
- `--allow-profile-mismatch` warns and marks JSON metadata.
- `--fingerprint` selects deterministic non-preferred realizations without changing preferred roots.
- `--profile-from` recreates the stored non-secret profile exactly.

### Diff meaning

- A function-body-only edit produces an implementation change.
- A signature edit produces a signature change.
- A formatting-only edit is textual, not semantic.
- A line insertion produces a collapsed location section.
- A new function and call produce node and edge additions.
- Community churn is classified as analysis.
- Manifest and artifact-registry churn is classified as metadata.
- `--topology-only` excludes implementation, attribute, location, analysis, and metadata changes.

### Determinism and durability

- Forward and reverse diffs swap old/new values and added/removed kinds symmetrically.
- Repeated JSON output is byte-identical.
- Identical roots produce an empty change array without additional node reads.
- Broken output streams stop scanning promptly.
- Publication and preferred selection remain atomic across injected failures.
- Reopening SQLite validates all named roots and stored profile provenance.

### End-to-end replay

Keep synthetic repositories in mandatory CI for deterministic coverage. Provide an optional real-repository replay script that reproduces the LevelDB scenarios:

- `78a352f..4a0c572`: body-only deadlock fix and location separation.
- `bfae97f..1d6e8d6`: Zstd symbols and call edges added.

The replay verifies independent graph exports against Prolly node and edge counts, JSON determinism, reverse symmetry, topology filtering, historical query selection, and a clean original checkout.

## Acceptance criteria

The mitigation is complete when:

1. Lazy and eager paths select profiles through the same resolver.
2. Reading a valid preferred realization leaves durable operational state unchanged.
3. Normal diff cannot compare unequal fingerprints.
4. Body-only changes are visible without treating line shifts as semantic changes.
5. Default output presents semantic changes first and collapses location, analysis, and metadata churn.
6. JSON remains complete, deterministic, streamable, and explicitly versioned.
7. All resolver, durability, classification, CLI, and real-replay verification gates pass.
