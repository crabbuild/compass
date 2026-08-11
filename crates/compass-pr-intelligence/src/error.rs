#[derive(Debug, thiserror::Error)]
pub enum PrIntelligenceError {
    #[error("unsupported PR Intelligence schema {0:?}")]
    UnsupportedSchema(String),
    #[error("invalid PR Intelligence evidence: {0}")]
    InvalidEvidence(String),
    #[error("PR Intelligence input exceeds limit: {0}")]
    Limit(String),
    #[error("could not encode canonical PR Intelligence JSON: {0}")]
    Json(#[from] serde_json::Error),
}
