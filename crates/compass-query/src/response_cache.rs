//! Disposable, bounded response cache. Every engine pins a verified graph;
//! request keys also include Program IR, query profiles and semantic mode.
use crate::{CodeQueryEngine, QueryError};
use rusqlite::{Connection, OptionalExtension, params};
use serde::{Serialize, de::DeserializeOwned};
use sha2::{Digest, Sha256};
use std::path::Path;
use std::sync::Mutex;

const MAX_PAYLOAD_BYTES: usize = 1024 * 1024;
const MAX_ENTRIES: usize = 32;

pub(crate) struct ResponseCache {
    connection: Mutex<Option<Connection>>,
    identity: String,
}

impl ResponseCache {
    #[cfg(test)]
    pub(crate) fn disabled() -> Self {
        Self {
            connection: Mutex::new(None),
            identity: String::new(),
        }
    }

    pub(crate) fn open(
        root: &Path,
        graph: &str,
        program: Option<&compass_ir::ProgramBundle>,
    ) -> Self {
        let mut digest = Sha256::new();
        digest.update(graph.as_bytes());
        digest.update(env!("CARGO_PKG_VERSION").as_bytes());
        digest.update(crate::QUERY_PLANNER_PROFILE_V2.as_bytes());
        digest.update(crate::QUERY_RANKER_PROFILE_V1.as_bytes());
        if let Some(program) = program {
            // Hash through a writer, without constructing another IR-sized buffer.
            if serde_json::to_writer(DigestWriter(&mut digest), program).is_err() {
                return Self {
                    connection: Mutex::new(None),
                    identity: String::new(),
                };
            }
        }
        let identity = format!("{:x}", digest.finalize());
        let connection = (|| {
            std::fs::create_dir_all(root).ok()?;
            let path = root.join("responses-v1.sqlite3");
            if std::fs::metadata(&path)
                .is_ok_and(|metadata| !metadata.is_file() || metadata.len() > 64 * 1024 * 1024)
                || path.is_symlink()
            {
                return None;
            }
            let connection = Connection::open(path).ok()?;
            connection
                .busy_timeout(std::time::Duration::from_millis(50))
                .ok()?;
            let page_size = connection
                .query_row("PRAGMA page_size", [], |row| row.get::<_, u64>(0))
                .ok()?;
            if !(512..=65536).contains(&page_size) {
                return None;
            }
            let max_pages = 64 * 1024 * 1024 / page_size;
            connection
                .pragma_update(None, "max_page_count", max_pages)
                .ok()?;
            connection
                .execute_batch(
                    "PRAGMA journal_mode=DELETE;
                CREATE TABLE IF NOT EXISTS responses_v1 (
                  key TEXT PRIMARY KEY, payload BLOB NOT NULL, checksum TEXT NOT NULL,
                  accessed INTEGER NOT NULL);",
                )
                .ok()?;
            Some(connection)
        })();
        Self {
            connection: Mutex::new(connection),
            identity,
        }
    }
    fn key<T: Serialize>(&self, operation: &str, semantic: bool, request: &T) -> Option<String> {
        let mut digest = Sha256::new();
        digest.update(self.identity.as_bytes());
        digest.update(operation.as_bytes());
        digest.update([u8::from(semantic)]);
        serde_json::to_writer(DigestWriter(&mut digest), request).ok()?;
        Some(format!("{:x}", digest.finalize()))
    }
    fn get<R: DeserializeOwned>(&self, key: &str) -> Option<R> {
        let mut state = self
            .connection
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let connection = state.as_mut()?;
        let (bytes, checksum): (Vec<u8>, String) = connection
            .query_row(
                "SELECT payload, checksum FROM responses_v1 WHERE key=?1 AND length(payload)<=?2",
                params![key, MAX_PAYLOAD_BYTES],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .optional()
            .ok()??;
        if checksum != payload_digest(key, &bytes) {
            return None;
        }
        let result = serde_json::from_slice(&bytes).ok()?;
        let _ = connection.execute("UPDATE responses_v1 SET accessed=(SELECT COALESCE(MAX(accessed),0)+1 FROM responses_v1) WHERE key=?1", [key]);
        Some(result)
    }
    fn put<R: Serialize>(&self, key: &str, response: &R) {
        let mut buffer = CacheBuffer(Vec::new());
        if serde_json::to_writer(&mut buffer, response).is_err() {
            return;
        }
        let bytes = buffer.0;
        let checksum = payload_digest(key, &bytes);
        let mut state = self
            .connection
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let Some(connection) = state.as_mut() else {
            return;
        };
        let Ok(transaction) = connection.transaction() else {
            return;
        };
        if transaction.execute("INSERT OR REPLACE INTO responses_v1 VALUES(?1,?2,?3,(SELECT COALESCE(MAX(accessed),0)+1 FROM responses_v1))", params![key, bytes, checksum]).is_err() { return; }
        if transaction.execute("DELETE FROM responses_v1 WHERE key IN (SELECT key FROM responses_v1 ORDER BY accessed DESC,key LIMIT -1 OFFSET ?1)", [MAX_ENTRIES]).is_err() { return; }
        let _ = transaction.commit();
    }
}
fn payload_digest(key: &str, bytes: &[u8]) -> String {
    let mut digest = Sha256::new();
    digest.update(key.as_bytes());
    digest.update(bytes);
    format!("{:x}", digest.finalize())
}
struct CacheBuffer(Vec<u8>);
impl std::io::Write for CacheBuffer {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        if self.0.len().saturating_add(bytes.len()) > MAX_PAYLOAD_BYTES {
            return Err(std::io::Error::other("cache payload bound"));
        }
        self.0.extend_from_slice(bytes);
        Ok(bytes.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}
struct DigestWriter<'a>(&'a mut Sha256);
impl std::io::Write for DigestWriter<'_> {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        self.0.update(bytes);
        Ok(bytes.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}
impl CodeQueryEngine {
    pub(crate) fn cached_response<T: Serialize, R: Serialize + DeserializeOwned + CacheRecord>(
        &self,
        operation: &str,
        request: &T,
        execute: impl FnOnce() -> Result<R, QueryError>,
        cacheable: impl Fn(&R) -> bool,
    ) -> Result<R, QueryError> {
        self.check_deadline()?;
        let key = self
            .response_cache
            .key(operation, self.semantic_search, request);
        if let Some(key) = &key
            && let Some(response) = self.response_cache.get::<R>(key)
        {
            self.check_deadline()?;
            if response.valid_cache_record() {
                return Ok(response);
            }
        }
        let response = execute()?;
        self.check_deadline()?;
        if cacheable(&response)
            && response.valid_cache_record()
            && let Some(key) = key
        {
            self.response_cache.put(&key, &response);
        }
        self.check_deadline()?;
        Ok(response)
    }
}

pub(crate) trait CacheRecord {
    fn valid_cache_record(&self) -> bool;
}
impl CacheRecord for compass_model::query_contract::CodeQueryResponse {
    fn valid_cache_record(&self) -> bool {
        self.schema == compass_model::query_contract::CODE_QUERY_SCHEMA_V1
            && !self.truncated
            && self.files.is_empty()
    }
}
impl CacheRecord for compass_model::query_contract::DiscoveryQueryResponse {
    fn valid_cache_record(&self) -> bool {
        self.schema == compass_model::query_contract::DISCOVERY_QUERY_SCHEMA_V1 && !self.truncated
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use compass_model::query_contract::{CodeQueryLimits, CodeQueryOperation, CodeQueryResponse};

    #[test]
    fn reopened_cache_is_bound_to_request_graph_and_mode_and_checks_corruption()
    -> Result<(), Box<dyn std::error::Error>> {
        let root = tempfile::tempdir()?;
        let cache = ResponseCache::open(root.path(), "graph-a", None);
        let key = cache.key("callers", false, &"target").ok_or("key")?;
        let response =
            CodeQueryResponse::empty(CodeQueryOperation::Callers, CodeQueryLimits::default());
        cache.put(&key, &response);
        drop(cache);
        let cache = ResponseCache::open(root.path(), "graph-a", None);
        assert_eq!(cache.get::<CodeQueryResponse>(&key), Some(response));
        for changed in [
            cache.key("callees", false, &"target"),
            cache.key("callers", false, &"other"),
            cache.key("callers", true, &"target"),
        ] {
            assert!(
                cache
                    .get::<CodeQueryResponse>(&changed.ok_or("key")?)
                    .is_none()
            );
        }
        let other = ResponseCache::open(root.path(), "graph-b", None);
        assert!(
            other
                .get::<CodeQueryResponse>(&other.key("callers", false, &"target").ok_or("key")?)
                .is_none()
        );
        let program = compass_ir::ProgramBundle::default();
        let different_ir = ResponseCache::open(root.path(), "graph-a", Some(&program));
        assert!(
            different_ir
                .get::<CodeQueryResponse>(
                    &different_ir.key("callers", false, &"target").ok_or("key")?
                )
                .is_none()
        );
        let connection = Connection::open(root.path().join("responses-v1.sqlite3"))?;
        connection.execute("UPDATE responses_v1 SET payload='{}' WHERE key=?1", [&key])?;
        assert!(cache.get::<CodeQueryResponse>(&key).is_none());
        Ok(())
    }

    #[test]
    fn bounded_cache_evicts_old_entries_and_skips_oversized_payloads()
    -> Result<(), Box<dyn std::error::Error>> {
        let root = tempfile::tempdir()?;
        let cache = ResponseCache::open(root.path(), "graph", None);
        for index in 0..MAX_ENTRIES + 5 {
            cache.put(&index.to_string(), &index);
        }
        assert!(cache.get::<usize>("0").is_none());
        assert_eq!(cache.get::<usize>("36"), Some(36));
        cache.put("oversized", &"x".repeat(MAX_PAYLOAD_BYTES + 1));
        assert!(cache.get::<String>("oversized").is_none());
        let connection = Connection::open(root.path().join("responses-v1.sqlite3"))?;
        assert_eq!(
            connection.query_row("SELECT COUNT(*) FROM responses_v1", [], |row| row
                .get::<_, usize>(0))?,
            MAX_ENTRIES
        );
        Ok(())
    }
    #[test]
    fn engine_hits_skip_execution_and_partial_responses_are_not_cached()
    -> Result<(), Box<dyn std::error::Error>> {
        use compass_model::code_graph::{BuildMetadata, GraphDocument};
        let root = tempfile::tempdir()?;
        let path = root.path().join("graph.json");
        let graph = GraphDocument::empty_v1(BuildMetadata {
            builder_version: "test".to_owned(),
            schema_fingerprint: "sha256:test".to_owned(),
            source_tree_digest: "sha256:test".to_owned(),
            configuration_digest: "sha256:test".to_owned(),
            generation_id: "sha256:test".to_owned(),
            source_commit: None,
        });
        std::fs::write(&path, serde_json::to_vec(&graph)?)?;
        let engine = crate::open(&path, None, &root.path().join("cache"))?;
        let executions = std::cell::Cell::new(0);
        let limits = CodeQueryLimits::default();
        for _ in 0..2 {
            engine.cached_response(
                "test",
                &limits,
                || {
                    executions.set(executions.get() + 1);
                    Ok(CodeQueryResponse::empty(
                        CodeQueryOperation::Callers,
                        limits.clone(),
                    ))
                },
                |_| true,
            )?;
        }
        assert_eq!(executions.get(), 1);
        for _ in 0..2 {
            engine.cached_response(
                "partial",
                &limits,
                || {
                    executions.set(executions.get() + 1);
                    let mut response =
                        CodeQueryResponse::empty(CodeQueryOperation::Callers, limits.clone());
                    response.truncated = true;
                    Ok(response)
                },
                |_| true,
            )?;
        }
        assert_eq!(executions.get(), 3);
        for _ in 0..2 {
            engine.cached_response(
                "source",
                &limits,
                || {
                    executions.set(executions.get() + 1);
                    let mut response =
                        CodeQueryResponse::empty(CodeQueryOperation::Callers, limits.clone());
                    response
                        .files
                        .push(compass_model::query_contract::QueryFile {
                            path: "src/lib.rs".to_owned(),
                            content_digest: "sha256:test".to_owned(),
                            source: Some("code".to_owned()),
                            truncated: false,
                        });
                    Ok(response)
                },
                |_| true,
            )?;
        }
        assert_eq!(executions.get(), 5);
        let different_limits = CodeQueryLimits {
            max_nodes: 1,
            ..limits
        };
        engine.cached_response(
            "test",
            &different_limits,
            || {
                executions.set(executions.get() + 1);
                Ok(CodeQueryResponse::empty(
                    CodeQueryOperation::Callers,
                    different_limits.clone(),
                ))
            },
            |_| true,
        )?;
        assert_eq!(executions.get(), 6);
        Ok(())
    }
}
