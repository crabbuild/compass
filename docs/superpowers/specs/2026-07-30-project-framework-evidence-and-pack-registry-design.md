# Project Framework Evidence and Pack Registry Design

**Date:** 2026-07-30

**Status:** Approved for implementation planning

**Implementation root:** `/Users/haipingfu/graphify/compass`

## Purpose

Compass framework extraction now distinguishes direct activation evidence from
supporting conventions, but activation remains file-local and framework
dispatch remains a hard-coded language match. This phase adds project-level
dependency evidence and a registry-driven dispatcher without changing the
public graph schema.

## Goals

- Read supported dependency manifests once per repository build.
- Resolve each source file to the nearest enclosing project root.
- Share immutable project evidence across parallel extraction workers.
- Prevent stale framework facts when a manifest changes but source files do
  not.
- Replace hard-coded framework dispatch with static, testable pack
  descriptors.
- Require project dependency evidence for convention-driven file routers when
  repository context is available.
- Preserve isolated `Engine::default()` extraction for callers without
  repository context.
- Keep activation work bounded by relevant languages, artifact types, and
  module dependencies.

## Non-goals

- Add a public or dynamically loaded plugin ABI.
- Add dozens of new framework detectors in this phase.
- Execute package managers, resolve lockfiles, or access the network.
- Infer transitive dependencies not declared by a local manifest.
- Change `RawFrameworkFact`, graph-v1 serialization, or route normalization.
- Move framework detection out of `compass-languages`.

## Project evidence index

`compass-languages` will expose an immutable `ProjectEvidenceIndex`. The core
pipeline builds it before AST cache lookup and shares it with each extraction
worker through `Arc`.

The index contains project entries keyed by normalized manifest directory:

```text
ProjectEvidence
  project_root
  manifests
  ecosystems
  normalized_dependencies
  fingerprint
```

Supported inputs in this phase are:

- `package.json`
- `composer.json`
- `pyproject.toml`
- `requirements.txt` and `requirements.in`
- `Gemfile`
- `pom.xml`
- `build.gradle` and `build.gradle.kts`
- `Cargo.toml`
- `go.mod`
- `*.csproj`
- `Package.swift`

The builder receives the repository root and detected source paths. It gathers
each source directory and its ancestors up to the root, probes every unique
directory once for recognized manifest names, and reads only regular,
non-symlink files within the repository. Manifest size and dependency-count
limits prevent unbounded input.

Dependencies are normalized conservatively:

- case-fold where the ecosystem is case-insensitive;
- remove version constraints without changing package identity;
- retain Maven group/artifact and Go module paths;
- retain scoped JavaScript package names; and
- deduplicate with deterministic ordering.

Malformed or oversized manifests contribute a bounded diagnostic entry and no
dependency evidence. They never activate a framework.

## Source-to-project resolution

The nearest manifest directory containing a source file owns that source.
Manifests in the same directory merge into one project entry. A source without
an enclosing manifest receives an explicit repository fallback entry with no
dependencies.

Lookup walks normalized ancestors and uses the prebuilt directory map; it does
not perform filesystem I/O during source extraction.

## Cache correctness

Framework facts are part of cached AST extraction, so source content alone is
not a sufficient cache key once manifests affect activation.

Each extraction created with repository context records its owning project
fingerprint in a private extension field:

```text
_compass_framework_project_evidence
```

Before accepting a cached AST extraction, the core pipeline compares the
stored fingerprint with the current index result for that source:

- equal fingerprints reuse the cached extraction;
- absent or different fingerprints re-extract that source; and
- only sources owned by the changed project are invalidated.

Fresh values replace their existing content-addressed cache entry. This keeps
current cache storage compatible while making project-evidence changes
correct. Cross-repository shared-cache collisions remain safe because a
fingerprint mismatch forces re-extraction.

The fingerprint covers the evidence schema version, normalized project root
relative to the repository, recognized manifest names, and normalized
dependencies. It excludes dependency versions because framework activation
in this phase depends only on declared package identity.

## Engine integration

`Engine` gains an optional shared project evidence index:

```text
Engine::default()
Engine::with_project_evidence(Arc<ProjectEvidenceIndex>)
```

Repository builds always use the second form. Isolated callers and existing
in-memory tests may continue using `Engine::default()`.

Framework detection receives an optional `ProjectEvidence` reference in its
detection context. No-context extraction preserves current exact local
activation behavior. When repository context is present, packs may require
manifest evidence in addition to local evidence.

## Framework pack registry

`frameworks/mod.rs` will define static descriptors rather than a language
`match`:

```text
FrameworkPack
  id
  languages
  artifact_kind
  dependency_markers
  manifest_policy
  detector
```

Artifact kinds are source, configuration, and template. Registry selection
first filters by artifact kind and language. Manifest policy is one of:

- advisory for code-driven packs, where dependencies may avoid unnecessary
  work but their absence cannot override an exact import or receiver;
- required for convention-driven packs such as file-system routers; or
- not applicable for framework-owned declarative configuration formats.

Exact construct activation remains the detector's responsibility.

Initial descriptors wrap existing language pack modules so this phase does
not duplicate or rewrite their construct parsers. A descriptor may represent
a related pack family, such as Python web frameworks or TypeScript routers.
New frameworks can later use one-framework descriptors.

Registry tests enforce:

- unique descriptor IDs;
- nonempty language or artifact matchers;
- deterministic declaration order;
- no duplicate detector execution for one artifact; and
- complete coverage of every detector currently invoked by
  `frameworks/mod.rs`.

## Activation policy changes

Project manifests are a distinct evidence kind. A declared dependency can
select a candidate pack but cannot independently turn an arbitrary call into a
framework fact.

For code-driven frameworks, exact import, receiver, decorator, attribute, or
macro evidence remains mandatory.

For file-system routers during repository builds:

- SvelteKit requires `@sveltejs/kit` plus the exact `src/routes` artifact
  contract;
- Nuxt requires `nuxt` plus its exact page/server artifact contract; and
- Astro requires `astro` plus its exact `src/pages` artifact contract.

Isolated extraction without a project index retains the existing exact
artifact-contract behavior for compatibility.

Framework-owned declarative formats such as Play `conf/routes` and Drupal
routing YAML remain direct configuration evidence and do not require a
separate package manifest.

## Performance

- Manifest discovery probes each unique source ancestor directory once.
- Manifest files are bounded by size and dependency count.
- The immutable index performs no worker-time filesystem I/O.
- Source lookup is proportional to path depth.
- Registry selection considers only matching artifact/language descriptors.
- Cached files are invalidated per owning project rather than repository-wide.
- Existing framework resolution scale ceilings remain release gates.

## Testing

Unit tests cover manifest parsing, normalization, nearest-project lookup,
deterministic fingerprints, malformed inputs, size limits, and symlink
rejection.

Pipeline tests cover:

- a file-router fact emitted with the matching project dependency;
- the same route convention rejected when the dependency is absent;
- a nested project overriding its parent manifest;
- a manifest-only change invalidating affected cached source facts;
- an unrelated project retaining its cached facts; and
- shared evidence across parallel workers.

Registry tests cover descriptor invariants and existing pack coverage.
Existing positive, negative, limit, serialization, and scale tests remain
green.

## Delivery sequence

1. Add `ProjectEvidenceIndex` and parser/lookup tests.
2. Add optional evidence context to `Engine`.
3. Add project fingerprints to extraction extensions.
4. Integrate module-scoped cache validation in `compass-core`.
5. Introduce source/config/template pack registries.
6. Make SvelteKit, Nuxt, and Astro repository builds project-aware.
7. Run focused tests, full resolver tests, strict Clippy, scale tests, and
   `graphify update .`.

## Acceptance criteria

- Repository extraction performs one bounded project-evidence build.
- Parallel workers share the same immutable index.
- Manifest changes cannot leave stale framework facts in cached source
  extractions.
- Existing framework modules are reached through registry descriptors.
- File-router conventions fail closed when repository evidence disproves the
  framework.
- Isolated extraction compatibility is preserved.
- Public graph contracts remain unchanged.
- Correctness, lint, and performance gates pass.
