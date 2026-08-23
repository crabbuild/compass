# Compass Graph Knowledge

This context names the source-derived graph and the verified knowledge that an
AI agent may add without rewriting Compass's structural truth.

## Language

**Base Graph**:
The validated, source-derived `compass.graph/1` artifact that remains authoritative and immutable for one build.
_Avoid_: Raw graph, original graph

**Base Generation**:
The exact generation identifier and canonical digest of a Base Graph to which an Agent Graph Overlay is bound.
_Avoid_: Current graph, latest graph

**Agent Assertion**:
A proposed node, edge, challenge, or retraction authored through an agent write adapter and owned by the Agent Graph Overlay. A proposal is not published until its Grounding succeeds.
_Avoid_: AI-inferred fact, synthetic fact

**Grounding**:
Deterministic verification that an Agent Assertion's bounded citations resolve to the stated Base Generation and retain valid content, identities, and source anchors. Grounding verifies evidence integrity; it does not claim universal truth.
_Avoid_: Model confidence, plausibility

**GROUNDED**:
The publication state earned by an Agent Assertion after Grounding succeeds. It is never accepted merely because an agent labels its own output as grounded.
_Avoid_: AI-inferred, evidenced, generated

**Agent Graph Overlay**:
A versioned, agent-owned set of GROUNDED assertions, challenges, and retractions that can be composed with a Base Graph without mutating it.
_Avoid_: Mutable graph, patched graph

**Overlay Revision**:
An immutable, canonically identified result of applying one atomic change batch to an Agent Graph Overlay.
_Avoid_: Session state, edit number

**Effective Graph**:
A deterministic read view of one Base Graph and one compatible Overlay Revision under an explicit composition policy.
_Avoid_: Merged truth, modified base graph

**Challenge**:
A GROUNDED Agent Assertion that disputes or qualifies a Base Graph fact while preserving that fact and its evidence.
_Avoid_: Delete, correction

**Retraction**:
An immutable tombstone that ends the active lifetime of an agent-owned assertion while preserving its prior revisions and audit history.
_Avoid_: Hard delete, erase

**Attestation**:
Minimal actor, session, adapter, and request metadata retained for audit but excluded from stable graph identity. Prompts, chain-of-thought, credentials, and full chat transcripts are not Attestations.
_Avoid_: Transcript, prompt log
