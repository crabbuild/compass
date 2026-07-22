//! Immutable, SQLite-backed version history for complete Compass graphs.

mod artifacts;
mod canonical;
mod config;
mod diff;
mod durable;
mod error;
mod fingerprint;
mod git;
mod jobs;
mod keys;
mod leases;
mod lock;
mod model;
mod store;
mod validate;

pub use artifacts::{CompletedGraphArtifacts, GraphArtifacts, PartitionedGraph};
pub use canonical::{CANONICAL_ENCODING_VERSION, canonical_json_bytes};
pub use config::HistoryConfig;
pub use diff::{ChangeKind, ChangeSink, GraphChange, RecordKind};
pub use error::HistoryError;
pub use fingerprint::{BuildProfile, ExtractionFingerprint, ExtractionFingerprintInput};
pub use git::{GitTargetLimitation, Repository, WorktreeGuard};
pub use jobs::{ClaimedJob, HistoryQueue, JobRecord, JobRequest, JobState};
pub use keys::{edge_key, hyperedge_key, node_key};
pub use leases::{LEASE_DURATION_MILLIS, LEASE_HEARTBEAT_MILLIS, LeaseGuard};
pub use lock::{ActivityGuard, MaintenanceGuard};
pub use model::{
    ArtifactClass, ArtifactContent, ArtifactRegistryEntry, CommitId, CompletionEvidence,
    GraphVersion, HISTORY_SCHEMA_VERSION, PublishRequest, PublishedVersion, RealizationId,
    StoredTree,
};
pub use store::{CorruptPreferredToken, HistoryStore, PreparedPublication};
pub use validate::{
    MAX_AUTHORITATIVE_BYTES, MAX_DIAGNOSTIC_BYTES, MAX_JOB_BYTES, MAX_JSON_DEPTH, MAX_KEY_BYTES,
    MAX_RECORD_VALUE_BYTES, MAX_RECORDS_PER_TREE, ValidationProblem, ValidationReport,
};
