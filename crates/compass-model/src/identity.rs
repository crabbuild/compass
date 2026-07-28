use sha2::{Digest, Sha256};

use crate::code_graph::{CODE_GRAPH_SCHEMA_V1, EdgeKind, NodeKind};
use crate::provenance::SourceAnchor;

const FILE_DOMAIN: &str = "file";
const SYMBOL_DOMAIN: &str = "symbol";
const ROUTE_DOMAIN: &str = "route";
const MESSAGING_DOMAIN: &str = "messaging";
const DATABASE_DOMAIN: &str = "database";
const DOMAIN_DOMAIN: &str = "domain";
const EDGE_DOMAIN: &str = "edge";

#[must_use]
pub fn normalize_repository_path(path: &str) -> String {
    let replaced = path.replace('\\', "/");
    let bytes = replaced.as_bytes();
    let without_drive = if bytes.len() >= 2 && bytes[0].is_ascii_alphabetic() && bytes[1] == b':' {
        &replaced[2..]
    } else {
        &replaced
    };
    let mut parts = Vec::new();
    for part in without_drive.split('/') {
        match part {
            "" | "." => {}
            ".." => {
                if parts.last().is_some_and(|previous| *previous != "..") {
                    parts.pop();
                } else {
                    parts.push(part);
                }
            }
            _ => parts.push(part),
        }
    }
    parts.join("/")
}

#[must_use]
pub fn file_id(normalized_path: &str) -> String {
    stable_id(FILE_DOMAIN, &[&normalize_repository_path(normalized_path)])
}

#[must_use]
pub fn symbol_id(
    language: &str,
    normalized_path: &str,
    kind: NodeKind,
    qualified_name: &str,
    disambiguator: &str,
) -> String {
    stable_id(
        SYMBOL_DOMAIN,
        &[
            language,
            &normalize_repository_path(normalized_path),
            kind.as_str(),
            qualified_name,
            disambiguator,
        ],
    )
}

#[must_use]
pub fn route_id(
    framework: &str,
    normalized_path: &str,
    operation: &str,
    route_path: &str,
    declaring_scope: &str,
) -> String {
    stable_id(
        ROUTE_DOMAIN,
        &[
            framework,
            &normalize_repository_path(normalized_path),
            &operation.to_ascii_uppercase(),
            route_path,
            declaring_scope,
        ],
    )
}

#[must_use]
pub fn messaging_id(
    kind: NodeKind,
    transport: &str,
    subject: &str,
    declaring_scope: &str,
) -> String {
    stable_id(
        MESSAGING_DOMAIN,
        &[kind.as_str(), transport, subject, declaring_scope],
    )
}

#[must_use]
pub fn database_entity_id(
    kind: NodeKind,
    logical_database: &str,
    database_schema: &str,
    qualified_name: &str,
) -> String {
    stable_id(
        DATABASE_DOMAIN,
        &[
            kind.as_str(),
            logical_database,
            database_schema,
            qualified_name,
        ],
    )
}

#[must_use]
pub fn domain_id(kind: NodeKind, namespace: &str, qualified_name: &str) -> String {
    stable_id(DOMAIN_DOMAIN, &[kind.as_str(), namespace, qualified_name])
}

#[must_use]
pub fn edge_id(
    source: &str,
    kind: EdgeKind,
    target: &str,
    relationship_site: Option<&SourceAnchor>,
    rule: Option<&str>,
) -> String {
    let anchor = relationship_site.map_or_else(String::new, canonical_anchor);
    stable_id(
        EDGE_DOMAIN,
        &[
            source,
            kind.as_str(),
            target,
            &anchor,
            rule.unwrap_or_default(),
        ],
    )
}

fn canonical_anchor(anchor: &SourceAnchor) -> String {
    format!(
        "{}:{}:{}:{}:{}:{}:{}",
        normalize_repository_path(&anchor.file),
        anchor.start_byte,
        anchor.end_byte,
        anchor.start_line,
        anchor.start_column,
        anchor.end_line,
        anchor.end_column
    )
}

fn stable_id(domain: &str, values: &[&str]) -> String {
    let mut hasher = Sha256::new();
    write_part(&mut hasher, CODE_GRAPH_SCHEMA_V1.as_bytes());
    write_part(&mut hasher, domain.as_bytes());
    for value in values {
        write_part(&mut hasher, value.as_bytes());
    }
    let digest = hasher.finalize();
    let mut encoded = String::with_capacity(71);
    encoded.push_str("sha256:");
    for byte in digest {
        use std::fmt::Write as _;
        let _ = write!(encoded, "{byte:02x}");
    }
    encoded
}

fn write_part(hasher: &mut Sha256, value: &[u8]) {
    hasher.update((value.len() as u64).to_be_bytes());
    hasher.update(value);
}
