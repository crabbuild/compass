#[derive(Debug, thiserror::Error)]
pub enum SemanticDiffError {
    #[error("semantic diff input is invalid: {0}")]
    InvalidInput(String),
    #[error("semantic evidence could not be read: {0}")]
    Evidence(String),
    #[error("semantic finding {0} does not exist")]
    FindingNotFound(String),
    #[error("semantic diff {resource} limit exceeded ({limit})")]
    LimitExceeded {
        resource: &'static str,
        limit: usize,
    },
    #[error("semantic report serialization failed: {0}")]
    Serialize(#[from] serde_json::Error),
}
