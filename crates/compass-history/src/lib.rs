//! Immutable, SQLite-backed version history for complete Compass graphs.

mod error;
mod git;
mod lock;
mod store;

pub use error::HistoryError;
pub use git::Repository;
pub use lock::{ActivityGuard, MaintenanceGuard};
pub use store::HistoryStore;
