# SurrealDB 3.2.4 license and release decision

Status: **APPROVED — ACCEPT**
Evidence date: 2026-08-28
Decision date: 2026-08-29
Decision scope: Compass artifacts that may depend on or contain SurrealDB 3.2.4

This is a factual release-gate record, not legal advice. The user accepted every
named artifact profile below under its stated conditions. That acceptance is
limited to the pinned SurrealDB 3.2.4 evidence and does not waive the remaining
technical, notice, redistribution, or independent-problem gates.

## Pinned evidence

The reviewed upstream tag is `v3.2.4`. Its exact
[`LICENSE`](https://raw.githubusercontent.com/surrealdb/surrealdb/v3.2.4/LICENSE)
has SHA-256:

```text
98a94ac615f88370865016487b436fa404560910bd329794ed7502277a94b805
```

The license parameters in that tagged file are:

| Parameter | Pinned value |
| --- | --- |
| License | Business Source License 1.1 |
| Licensor | SurrealDB Ltd. |
| Licensed Work | SurrealDB 3.0, copyright 2025 SurrealDB Limited |
| Additional Use Grant | Use is allowed except use as a Database Service |
| Database Service boundary | A product, service, platform, or commercial offering that provides database functionality to third parties (other than direct employees or contractors) and enables those third parties to create, manage, or control schemas or tables |
| Change Date | 2030-01-01 |
| Change License | Apache License, Version 2.0 |

The license converts on the earlier of the stated Change Date and the fourth
anniversary of the first publicly available distribution of the specific
version under this license.

The BSL terms also state that copies and derivative works remain under the
license until conversion, the license applies separately per version, and the
license must be conspicuously displayed on original or modified copies.

The tagged [`surrealdb` crate
manifest](https://raw.githubusercontent.com/surrealdb/surrealdb/v3.2.4/surrealdb/Cargo.toml)
publishes with `license-file` metadata and an unconditional dependency on
`surrealdb-core`. Its HTTP and WebSocket transports are features, but selecting
only a remote protocol does not remove that core dependency. The tagged
[`surrealdb-core` manifest](https://raw.githubusercontent.com/surrealdb/surrealdb/v3.2.4/surrealdb/core/Cargo.toml)
publishes with the same workspace license file and owns the embedded Mem,
SurrealKV, and RocksDB features.

SurrealDB's [official licensing
FAQ](https://github.com/surrealdb/license) describes embedding, modification,
redistribution, and production use as permitted by its Additional Use Grant,
while excluding commercial DBaaS use and noting that the BSL is not an
OSI-approved open-source license before conversion. The tagged license text,
not the FAQ summary, controls this record.

## Artifact profile

| Compass artifact | BSL/core relationship | Release condition if approved |
| --- | --- | --- |
| Source checkout with no Surreal dependency | Contains neither Surreal core nor its dependency metadata | No Surreal-specific notice. |
| Optional Compass crate on crates.io | The Compass crate need not copy core source, but activating it resolves BSL `surrealdb` and `surrealdb-core` packages | Pin 3.2.4, disclose the optional BSL dependency and Database Service boundary, and require downstream artifacts to carry applicable notices. |
| Prebuilt binary or native library with embedded support | Links covered core code | Bundle the exact tagged license conspicuously, identify the covered version and restriction, retain notices, and do not label the complete artifact as exclusively OSI-open-source. |
| Plugin or archive containing such a binary | Redistributes the same covered bytes | Apply the prebuilt-binary conditions to the archive and its package metadata. |
| Container containing a Surreal-enabled Compass binary or SurrealDB server | Redistributes covered code | Include the exact license and version in the image and accompanying notices; disclose the restriction at the distribution surface. |
| Codex, Claude, or OpenCode skill/config package only | No covered bytes when it contains only prose and configuration | No Surreal notice solely for the package; generation must not silently attach a covered executable. |
| Downstream redistribution of covered source or binaries | Redistributor receives a covered copy | Preserve and conspicuously display the license, notices, version, Change Date, Change License, and Database Service restriction. |
| Genuinely remote-client-only integration | No BSL core linked into Compass | Use a core-free protocol implementation and an independently installed server. The official Rust SDK 3.2.4 is not treated as core-free because its manifest retains the unconditional core dependency. |

Cargo feature isolation contains default build and binary footprint. It does
not erase obligations from a Surreal-enabled artifact that resolves or embeds
the covered core.

## Outcome effects

- **Accept all profiles:** C-014 and C-015 may proceed after C-012 and C-013
  also pass. C-020 still requires its separate user-problem condition.
- **Conditional:** only explicitly named profiles may proceed. All omitted
  profiles are prohibited, and later changes must encode the allowed set as a
  release gate.
- **Reject:** cancel C-014 and C-015 as framed and cancel the Surreal branch of
  C-020. Retain SQLite/redb/`graph.json`. A later remote-only proposal may be
  evaluated only after dependency-tree evidence proves Compass links no BSL
  core.

## Sign-off

```text
Decision: ACCEPT
Approved profiles: Every artifact profile named in this record, under its stated release conditions
Prohibited profiles: None beyond the conditions and Database Service restriction recorded above
Decision authority/name: Project user / active Codex task decision authority
Authority role (user or legal reviewer): User
Decision date: 2026-08-29
Provenance (task message, review reference, or signed record): Active Codex task message: "C-011: ACCEPT"
Conditions or notes: Applies only to pinned SurrealDB 3.2.4; preserve the exact BSL license, notices, version, Change Date, Change License, and Database Service restriction for covered artifacts and redistribution; perform a fresh license/profile review for every version upgrade
```
