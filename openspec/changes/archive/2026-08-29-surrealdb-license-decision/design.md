## Context

SurrealDB 3.2.4's official tagged `LICENSE` identifies the work as SurrealDB
3.0 under Business Source License 1.1, licensed by SurrealDB Ltd. Its Additional
Use Grant permits use except as a defined Database Service, its Change Date is
2030-01-01, and its Change License is Apache License 2.0. Conversion occurs on
the earlier of that Change Date and the fourth anniversary of the first publicly
available distribution of the specific version under this license. The BSL
terms require the license to be conspicuously displayed on original or modified
copies and apply separately to each version.

The official 3.2.4 `surrealdb` crate is published with `license-file` metadata
and depends unconditionally on `surrealdb-core`; selecting only HTTP or WebSocket
protocol features does not by itself produce a core-free Rust SDK dependency.
The exact tagged license bytes have SHA-256
`98a94ac615f88370865016487b436fa404560910bd329794ed7502277a94b805`.

Primary evidence:

- <https://raw.githubusercontent.com/surrealdb/surrealdb/v3.2.4/LICENSE>
- <https://raw.githubusercontent.com/surrealdb/surrealdb/v3.2.4/surrealdb/Cargo.toml>
- <https://raw.githubusercontent.com/surrealdb/surrealdb/v3.2.4/surrealdb/core/Cargo.toml>
- <https://github.com/surrealdb/license>

This record summarizes licensing evidence for a release decision; it is not
legal advice.

## Goals / Non-Goals

**Goals:**

- Make every proposed distribution profile explicit before code or dependency
  work begins.
- Separate source dependency metadata from artifacts that actually contain BSL
  core bytes.
- Define notice, version-pin, and downstream redistribution conditions.
- Preserve a credible fallback when BSL-bearing artifacts are unacceptable.

**Non-Goals:**

- Interpret an individual customer's use as compliant or non-compliant.
- Grant trademark rights or replace counsel.
- Add SurrealDB, run engine probes, or approve production use.

## Decisions

### Evidence is pinned to 3.2.4

The release decision applies only to SurrealDB 3.2.4 and its exact tagged
license digest. A version bump must repeat the license comparison because BSL
parameters and Change Dates apply per version.

### Artifact profiles are evaluated independently

| Profile | Contains or induces BSL core? | Required release treatment |
| --- | --- | --- |
| Compass source repository with no Surreal dependency | No | No Surreal notice; Waves 5/8 remain absent. |
| Optional Compass crate published to crates.io | The Compass crate need not copy core source, but activating its dependency resolves the BSL `surrealdb`/`surrealdb-core` crates | Declare the optional BSL dependency prominently, pin the reviewed version, retain Compass's own license metadata, and require the consuming build/distribution to satisfy the Surreal license. |
| Prebuilt Compass binary, library, plugin, or archive with embedded Surreal support | Yes, linked core code | Ship the exact BSL license conspicuously with the artifact, identify SurrealDB 3.2.4 and the DB-service restriction, preserve notices, and do not describe the whole artifact as exclusively OSI-open-source. |
| Container image containing a Surreal-enabled Compass build or SurrealDB server | Yes | Apply the same BSL notice and restriction disclosure to the image and its accompanying materials; keep the covered version discoverable. |
| Skill/config-only Codex, Claude, or OpenCode package | No, provided it contains no Surreal-enabled binary or core source | No Surreal notice is needed solely for configuration or prose; generation must not silently add a covered binary. |
| Downstream redistribution of any covered binary/source copy | Yes | Redistributor must retain and conspicuously display the exact license, notices, version, Change Date, and Change License; the Database Service restriction remains in force until conversion. |
| Remote-client-only Compass integration with no `surrealdb` or `surrealdb-core` linkage | No core copy in Compass | Use a core-free protocol implementation and require an independently installed external server. Do not call the official Rust SDK profile core-free while its manifest retains the unconditional core dependency. |

### Approval is explicit and profile-scoped

The final record must contain one of:

- `accept`: every named profile above is approved under its stated conditions;
- `conditional`: only an enumerated subset is approved, with all other profiles
  prohibited; or
- `reject`: no Compass artifact may add the BSL core dependency or bytes.

Silence, an implementation commit, a successful probe, or an optional Cargo
feature is not approval.

### Rejection and conditional fallback

If embedded/core-bearing profiles are rejected, C-014 and C-015 are cancelled
as currently framed and the Surreal branch of C-020 is cancelled. Compass keeps
SQLite/redb/`graph.json` as the local authoritative stack. A later remote-only
proposal may proceed only if it proves that Compass links no BSL core and treats
the external SurrealDB installation as a separately licensed operator choice.

## Risks / Trade-offs

- **License text or crate topology changes in a later SurrealDB release** → Pin
  3.2.4 and require a fresh digest/profile review for every upgrade.
- **“Optional” is mistaken for “license-free”** → Evaluate distributed bytes
  and dependency resolution per profile; never use feature gating as a legal
  conclusion.
- **A remote profile still links core through the official Rust SDK** → Require
  dependency-tree evidence for a core-free fallback before calling it such.
- **Downstream users infer legal advice from project docs** → State the factual
  license parameters, conditions, and approval provenance without adjudicating
  a user's business model.

## Migration Plan

No runtime migration occurs. After explicit sign-off, copy the decision and its
provenance into the phase decision log. Accepted or conditional profiles become
hard constraints for later OpenSpec changes; rejection cancels the gated waves.
