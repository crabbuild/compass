# Agent Graph contract fixtures

These fixtures freeze the additive version-1 Agent Graph contracts. Request
fixtures never contain `GROUNDED`; only Compass-produced overlay/effective
outputs may contain that state.

Identity domains:

```text
Base Generation = (build generation ID, canonical compass.graph/1 SHA-256)
Overlay Revision = SHA-256(domain || canonical revision manifest)
Effective Graph  = SHA-256(domain || Base Generation || Overlay Revision ||
                          profile || composition version || canonical semantics)
```

All digests are lowercase SHA-256. All write targets are exact IDs or an
Assertion Key created in the same atomic batch. Unknown fields, unknown major
versions, fuzzy targets, caller-supplied certificates, and caller-supplied
principals are invalid.

`ingestion-preparation-v1.json` is a read-only Compass-produced response. It
pins the Base Generation and active Overlay Revision, calculates canonical Base
record digests, and turns repository-relative byte spans into complete source
evidence. Agents copy these values into a batch; apply still re-verifies every
citation before publication.

`audit-v1.json` is Compass-produced operational metadata. It records bounded
digests and trusted adapter/model labels, never prompts, responses,
chain-of-thought, credentials, tokens, or source excerpts.

The checked limits are recorded in `limits-v1.json`; runtime grants may lower
defaults but cannot exceed ceilings.
