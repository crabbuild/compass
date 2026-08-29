# Refinement decisions — C-014

- Keep `compass-model` as the canonical graph authority and make SurrealDB an
  additive projection crate with no CLI or MCP route in this change.
- Pin every engine profile to the user-approved SurrealDB 3.2.4 release with
  default SDK features disabled and exact feature isolation.
- Preserve exact canonical node and edge JSON alongside indexed typed fields so
  round trips retain identities, direction, multiplicity, source anchors,
  provenance, and confidence without interpreting unknown attributes.
- Use one immutable generation transaction for staging, validation, manifest
  publication, and repository-pointer activation; cancellation cannot expose a
  candidate generation.
- Restrict execution to closed static statements and parameter-bound values.
- Make finite limits part of each engine client. The 1,000,000-node,
  2,500,000-relation, and canonical `GraphDocument` reader 1 GiB serialized-byte
  defaults are independent ceilings, not a promise that every maximum-sized
  record population fits under the byte ceiling. C-015 owns qualification-corpus
  measurement; constrained callers may select smaller positive limits.
- Leave native multi-hop reads and dual-engine semantic qualification to C-015,
  as required by the phase dependency graph.
