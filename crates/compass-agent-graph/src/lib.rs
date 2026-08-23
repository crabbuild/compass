#![forbid(unsafe_code)]

//! Grounded, agent-authored graph assertions layered over an immutable Compass graph.
//!
//! This crate owns the semantic boundary between untrusted agent drafts and
//! publishable overlay state. It deliberately does not invoke a model, mutate a
//! base graph, or add `GROUNDED` to structural confidence.

mod assertion;
mod audit;
mod canonical;
mod compose;
mod contract;
mod grounding;
mod limits;
mod maintenance;
mod overlay;
mod paths;
mod policy;
mod rebase;
mod repository;

pub use assertion::{
    AgentEdgeDraft, AgentFactDraft, AgentNodeDraft, AssertionDraft, AssertionSelector,
    ChallengeDraft, ChallengeEffect, ChallengeSelector, ChangeBatch, ChangeOperation, NodeRef,
};
pub use audit::{
    AGENT_GRAPH_AUDIT_RESULT_SCHEMA_V1, AGENT_GRAPH_AUDIT_SCHEMA_V1, AuditOutcome, AuditRecord,
    AuditResult, WriteAttestation,
};
pub use canonical::{Digest, canonical_bytes, canonical_digest};
pub use compose::{
    AGENT_GRAPH_EFFECTIVE_SCHEMA_V1, CompositionOmission, CompositionOmissions, CompositionProfile,
    EffectiveAgentFact, EffectiveChallenge, EffectiveGraph, EffectiveRetraction,
    EffectiveRetractionKind, EffectiveRetractions, compose_effective,
};
pub use contract::{
    AGENT_GRAPH_BATCH_SCHEMA_V1, AGENT_GRAPH_RECEIPT_SCHEMA_V1, AgentGraphDiagnostic,
    AgentGraphError, AgentGraphErrorCode, AssertionDigest, AssertionId, AssertionKey, BaseEdgeRef,
    BaseFactRef, BaseGenerationId, BaseNodeRef, ChallengeId, CommitReceipt,
    GroundingCertificateDigest, IdempotencyKey, OverlayId, OverlayRevisionId, PinId, PrincipalId,
    RepositoryId,
};
pub use grounding::{
    ArtifactRecord, BaseGenerationView, GroundedAssertion, GroundedChallenge, GroundedEffect,
    GroundingCertificate, GroundingEvidence, GroundingPolicy, GroundingStatus, GroundingSubmission,
    InMemoryBaseGeneration, PriorAssertionRecord, ground_assertion, ground_challenge,
};
pub use limits::{AgentGraphLimits, HardLimits};
pub use maintenance::{GcAuthority, GcPlan, GcReceipt, QuiescentGcGrant, RevisionPin};
pub use overlay::{
    ActiveAssertion, ActiveChallenge, IdempotencyRecord, OverlayRevision, OverlayState,
    ResolvedAgentEdge, ResolvedAgentFact, ResolvedNodeRef, Retraction,
};
pub use paths::{AGENT_GRAPH_DATABASE_NAME, AgentGraphPaths};
pub use policy::{OperationPermission, WriteAuthority, WriteGrant};
pub use rebase::{
    AGENT_GRAPH_REBASE_COMMIT_SCHEMA_V1, AGENT_GRAPH_REBASE_PLAN_SCHEMA_V1, RebaseCommitRequest,
    RebaseDisposition, RebaseItem, RebasePlan, RebaseSubject,
};
pub use repository::{
    AgentGraphOverlay, BaseGenerationProvider, DiffResult, HistoryResult,
    InMemoryBaseGenerationProvider, OverlayRepository, ReadRequest, ReadResult,
};
