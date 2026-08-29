use serde_json::Value;

use crate::HistoryError;

pub use compass_partition::CANONICAL_ENCODING_VERSION;

/// Encode JSON into deterministic, whitespace-free UTF-8 bytes.
pub fn canonical_json_bytes(value: &Value) -> Result<Vec<u8>, HistoryError> {
    compass_partition::canonical_json_bytes(value).map_err(HistoryError::from)
}
