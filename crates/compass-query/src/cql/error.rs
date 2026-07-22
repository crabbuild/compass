use thiserror::Error;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum QueryErrorKind {
    InvalidParameter,
    Type,
    Arithmetic,
    Regex,
    RowLimit,
    ExpansionLimit,
    MemoryLimit,
    Timeout,
    Cancelled,
    Internal,
}

#[derive(Clone, Debug, Error)]
#[error("{code}: {message}")]
pub struct QueryError {
    kind: QueryErrorKind,
    code: &'static str,
    message: String,
}

impl QueryError {
    #[must_use]
    pub fn new(kind: QueryErrorKind, code: &'static str, message: impl Into<String>) -> Self {
        Self {
            kind,
            code,
            message: message.into(),
        }
    }

    #[must_use]
    pub const fn kind(&self) -> QueryErrorKind {
        self.kind
    }

    #[must_use]
    pub const fn code(&self) -> &'static str {
        self.code
    }

    #[must_use]
    pub fn message(&self) -> &str {
        &self.message
    }
}
