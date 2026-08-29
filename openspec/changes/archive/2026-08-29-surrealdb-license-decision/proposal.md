## Why

Compass must not add or distribute SurrealDB core until the exact BSL 1.1
artifact and redistribution profile has explicit release/legal approval. Making
that decision before probes and adapters prevents implementation momentum from
silently deciding a licensing question.

## What Changes

- Capture the exact SurrealDB 3.2.4 license parameters and immutable source
  evidence.
- Define the licensing effect of source, registry, prebuilt binary, plugin,
  container, and downstream-redistribution profiles.
- Record one explicit `accept`, `reject`, or `conditional` decision with named
  provenance and permitted profiles.
- Preserve a remote-client-only/external-server fallback that links no
  SurrealDB BSL core if the embedded profile is not accepted.

## Capabilities

### New Capabilities

None. This change records a release/legal decision and does not change shipped
product behavior; `.openspec.yaml` therefore opts out of delta specs.

### Modified Capabilities

None.

## Impact

The decision gates C-014, C-015, and the SurrealDB branch of C-020. It may
constrain future Cargo features, crates.io publication, prebuilt binaries,
plugin archives, containers, notices, and downstream redistribution, but this
change itself adds no dependency and changes no runtime behavior.
