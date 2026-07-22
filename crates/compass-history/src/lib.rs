//! Immutable, SQLite-backed version history for complete Compass graphs.

mod canonical;
mod error;
mod fingerprint;
mod git;
mod keys;
mod lock;
mod store;

pub use canonical::{CANONICAL_ENCODING_VERSION, canonical_json_bytes};
pub use error::HistoryError;
pub use fingerprint::{BuildProfile, ExtractionFingerprint, ExtractionFingerprintInput};
pub use git::Repository;
pub use keys::{edge_key, hyperedge_key, node_key};
pub use lock::{ActivityGuard, MaintenanceGuard};
pub use store::HistoryStore;
