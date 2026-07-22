//! Bounded, native Neo4j Bolt and FalkorDB RESP exporters.

mod bolt;
mod queries;
mod resp;

use std::collections::BTreeMap;

use compass_model::GraphDocument;

pub use queries::{CypherOperation, GraphOperations, falkordb_query, graph_operations};

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct PushCounts {
    pub nodes: usize,
    pub edges: usize,
}

#[derive(Debug, thiserror::Error)]
pub enum GraphDbError {
    #[error("invalid graph database URI")]
    InvalidUri,
    #[error("{0}")]
    Connection(&'static str),
    #[error("graph database I/O failed: {0}")]
    Socket(std::io::Error),
    #[error("graph database TLS failed: {0}")]
    Tls(rustls::Error),
    #[error("graph database protocol error: {0}")]
    Protocol(&'static str),
    #[error("Neo4j {stage} failed: {message}")]
    Neo4jResponse {
        stage: &'static str,
        message: String,
    },
    #[error("FalkorDB {stage} failed: {message}")]
    FalkorResponse {
        stage: &'static str,
        message: String,
    },
    #[error("{0}")]
    Sanitized(String),
}

pub fn push_to_neo4j(
    document: &GraphDocument,
    uri: &str,
    user: &str,
    password: &str,
    communities: Option<&BTreeMap<usize, Vec<String>>>,
) -> Result<PushCounts, GraphDbError> {
    let operations = graph_operations(document, communities);
    let embedded_password = uri_password(uri);
    let mut secrets = vec![password, uri];
    if let Some(embedded_password) = embedded_password.as_deref() {
        secrets.push(embedded_password);
    }
    bolt::push(&operations, uri, user, password).map_err(|error| sanitize(error, &secrets))
}

pub fn push_to_falkordb(
    document: &GraphDocument,
    uri: &str,
    user: Option<&str>,
    password: Option<&str>,
    communities: Option<&BTreeMap<usize, Vec<String>>>,
    graph_name: &str,
) -> Result<PushCounts, GraphDbError> {
    let operations = graph_operations(document, communities);
    let embedded_password = uri_password(uri);
    let mut secrets = vec![uri];
    if let Some(password) = password {
        secrets.push(password);
    }
    if let Some(embedded_password) = embedded_password.as_deref() {
        secrets.push(embedded_password);
    }
    resp::push(&operations, uri, user, password, graph_name)
        .map_err(|error| sanitize(error, &secrets))
}

fn uri_password(uri: &str) -> Option<String> {
    let normalized = if uri.contains("://") {
        uri.to_owned()
    } else {
        format!("bolt://{uri}")
    };
    url::Url::parse(&normalized)
        .ok()?
        .password()
        .map(str::to_owned)
}

fn sanitize(error: GraphDbError, secrets: &[&str]) -> GraphDbError {
    let mut message = error.to_string();
    for secret in secrets.iter().filter(|secret| !secret.is_empty()) {
        message = message.replace(secret, "[redacted]");
    }
    GraphDbError::Sanitized(message)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sanitization_covers_flag_and_uri_passwords() {
        let error = GraphDbError::Neo4jResponse {
            stage: "authentication",
            message: "flag-secret and uri-secret".to_owned(),
        };
        let uri_secret = uri_password("bolt://admin:uri-secret@example.test").unwrap_or_default();
        let sanitized = sanitize(error, &["flag-secret", &uri_secret]).to_string();
        assert!(!sanitized.contains("flag-secret"));
        assert!(!sanitized.contains("uri-secret"));
        assert!(sanitized.contains("[redacted]"));
    }
}
