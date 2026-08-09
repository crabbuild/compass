use std::cell::RefCell;
use std::collections::HashMap;
use std::sync::Arc;

use prolly::{Prolly, RuntimeConfig, Tree};
use prolly_store_sqlite::SqliteStore;

use crate::{
    ActivityGuard, HistoryError, HistoryRecord, HistoryRecordKey, HistoryStore, PublishedVersion,
    RealizationId, StoredTree,
};

const READER_NODE_CACHE_MAX_NODES: usize = 16_384;
const READER_NODE_CACHE_MAX_BYTES: usize = 128 * 1024 * 1024;
const READER_READ_PARALLELISM: usize = 16;

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
enum OwnedHistoryRecordKey {
    Node(String),
    ProgramModule(String),
    ProgramFunction(String),
    ProgramSummary(String),
    ReverseCallers(String),
}

impl From<HistoryRecordKey<'_>> for OwnedHistoryRecordKey {
    fn from(value: HistoryRecordKey<'_>) -> Self {
        match value {
            HistoryRecordKey::Node(value) => Self::Node(value.to_owned()),
            HistoryRecordKey::ProgramModule(value) => Self::ProgramModule(value.to_owned()),
            HistoryRecordKey::ProgramFunction(value) => Self::ProgramFunction(value.to_owned()),
            HistoryRecordKey::ProgramSummary(value) => Self::ProgramSummary(value.to_owned()),
            HistoryRecordKey::ReverseCallers(value) => Self::ReverseCallers(value.to_owned()),
        }
    }
}

/// One sealed realization opened for repeated typed reads and diffs.
pub struct RealizationReader<'store> {
    pub(crate) store: &'store HistoryStore,
    _activity: ActivityGuard,
    pub(crate) published: PublishedVersion,
    pub(crate) prolly: Prolly<Arc<SqliteStore>>,
    roots: RefCell<HashMap<&'static str, Tree>>,
    records: RefCell<HashMap<OwnedHistoryRecordKey, Option<HistoryRecord>>>,
}

impl HistoryStore {
    pub fn reader(
        &self,
        realization: &RealizationId,
    ) -> Result<RealizationReader<'_>, HistoryError> {
        let activity = self.activity()?;
        let published = self.get_with_activity(realization, &activity)?;
        let mut config = self.prolly.config().clone();
        config.runtime = RuntimeConfig {
            node_cache_max_nodes: Some(READER_NODE_CACHE_MAX_NODES),
            node_cache_max_bytes: Some(READER_NODE_CACHE_MAX_BYTES),
            read_parallelism: READER_READ_PARALLELISM,
        };
        Ok(RealizationReader {
            store: self,
            _activity: activity,
            published,
            prolly: Prolly::new(self.prolly.store().clone(), config),
            roots: RefCell::new(HashMap::new()),
            records: RefCell::new(HashMap::new()),
        })
    }
}

impl RealizationReader<'_> {
    #[must_use]
    pub fn version(&self) -> &PublishedVersion {
        &self.published
    }

    /// Load the exact trusted typed graph retained by this immutable realization.
    ///
    /// Compatibility-only historical projections are deliberately rejected:
    /// callers requiring `compass.graph/1` must rebuild older realizations.
    pub fn graph_document(&self) -> Result<compass_model::code_graph::GraphDocument, HistoryError> {
        let artifacts = self
            .store
            .artifacts_with_activity(&self.published.id, &self._activity)?;
        artifacts
            .artifacts
            .trusted_graph_document()?
            .ok_or_else(|| HistoryError::TrustedGraphUnavailable {
                realization: self.published.id.to_string(),
            })
    }

    pub fn read(&self, key: HistoryRecordKey<'_>) -> Result<Option<HistoryRecord>, HistoryError> {
        let owned = OwnedHistoryRecordKey::from(key);
        if let Some(value) = self.records.borrow().get(&owned) {
            return Ok(value.clone());
        }
        let (root_name, tree, encoded_key, schema) = match &owned {
            OwnedHistoryRecordKey::Node(id) => (
                "nodes",
                self.tree(&self.published.version.nodes_root),
                crate::node_key(id),
                "compass.node",
            ),
            OwnedHistoryRecordKey::ProgramModule(source_file) => (
                "program-facts",
                self.tree(&self.published.version.program_facts_root),
                crate::artifacts::program_key("module", source_file),
                "compass.program.module",
            ),
            OwnedHistoryRecordKey::ProgramFunction(symbol_id) => (
                "program-facts",
                self.tree(&self.published.version.program_facts_root),
                crate::artifacts::program_key("function", symbol_id),
                "compass.program.function",
            ),
            OwnedHistoryRecordKey::ProgramSummary(symbol_id) => (
                "program-summaries",
                self.tree(&self.published.version.program_summaries_root),
                crate::artifacts::program_key("summary", symbol_id),
                "compass.program.summary",
            ),
            OwnedHistoryRecordKey::ReverseCallers(symbol_id) => (
                "program-summaries",
                self.tree(&self.published.version.program_summaries_root),
                crate::artifacts::program_key("reverse-call", symbol_id),
                "compass.program.reverse-call",
            ),
        };
        let tree = self
            .roots
            .borrow_mut()
            .entry(root_name)
            .or_insert(tree)
            .clone();
        let value = match self.prolly.get(&tree, &encoded_key)? {
            None => None,
            Some(bytes) => {
                if bytes.len() > crate::MAX_RECORD_VALUE_BYTES {
                    return Err(HistoryError::CorruptHistory(
                        "history record exceeds byte limit".to_owned(),
                    ));
                }
                Some(match &owned {
                    OwnedHistoryRecordKey::Node(_) => {
                        HistoryRecord::Node(crate::artifacts::decode_compatible_node(&bytes)?)
                    }
                    OwnedHistoryRecordKey::ProgramModule(_) => {
                        let module = match crate::artifacts::decode_program_module(&bytes)? {
                            crate::artifacts::DecodedProgramModule::Embedded(module) => module,
                            crate::artifacts::DecodedProgramModule::Indexed(indexed) => {
                                hydrate_indexed_module(indexed, |symbol_id| {
                                    match self.read(HistoryRecordKey::ProgramFunction(symbol_id))? {
                                        Some(HistoryRecord::ProgramFunction(function)) => {
                                            Ok(function)
                                        }
                                        _ => Err(HistoryError::CorruptHistory(format!(
                                            "program module references missing function {symbol_id}"
                                        ))),
                                    }
                                })?
                            }
                        };
                        HistoryRecord::ProgramModule(module)
                    }
                    OwnedHistoryRecordKey::ProgramFunction(_) => HistoryRecord::ProgramFunction(
                        crate::artifacts::decode_typed(&bytes, schema)?,
                    ),
                    OwnedHistoryRecordKey::ProgramSummary(_) => HistoryRecord::ProgramSummary(
                        crate::artifacts::decode_typed(&bytes, schema)?,
                    ),
                    OwnedHistoryRecordKey::ReverseCallers(_) => HistoryRecord::ReverseCallers(
                        crate::artifacts::decode_typed(&bytes, schema)?,
                    ),
                })
            }
        };
        self.records.borrow_mut().insert(owned, value.clone());
        Ok(value)
    }

    pub fn read_many<'key>(
        &self,
        keys: impl IntoIterator<Item = HistoryRecordKey<'key>>,
    ) -> Result<Vec<Option<HistoryRecord>>, HistoryError> {
        keys.into_iter().map(|key| self.read(key)).collect()
    }

    pub(crate) fn tree(&self, stored: &StoredTree) -> Tree {
        stored.to_tree()
    }
}

fn hydrate_indexed_module(
    mut indexed: crate::artifacts::IndexedProgramModule,
    mut read_function: impl FnMut(&str) -> Result<compass_ir::FunctionIr, HistoryError>,
) -> Result<compass_ir::ModuleIr, HistoryError> {
    indexed.module.functions = indexed
        .function_ids
        .iter()
        .map(|symbol_id| read_function(symbol_id))
        .collect::<Result<Vec<_>, _>>()?;
    Ok(indexed.module)
}

#[cfg(test)]
mod tests {
    use compass_ir::{ExecutionMode, FunctionIr, ModuleIr, SourceAnchor, Visibility, hex_sha256};

    use super::hydrate_indexed_module;
    use crate::artifacts::IndexedProgramModule;

    #[test]
    fn indexed_module_reads_functions_in_declared_order() -> Result<(), crate::HistoryError> {
        let function = |symbol_id: &str| FunctionIr {
            symbol_id: symbol_id.to_owned(),
            name: symbol_id.to_owned(),
            graph_node_id: None,
            signature_digest: hex_sha256(symbol_id.as_bytes()),
            body_digest: hex_sha256(b"body"),
            visibility: Visibility::Public,
            execution_mode: ExecutionMode::Sync,
            is_test: false,
            anchor: SourceAnchor {
                source_file: "src/lib.rs".to_owned(),
                start_byte: 0,
                end_byte: 1,
            },
            parameters: Vec::new(),
            return_type: None,
            blocks: Vec::new(),
            coverage: Default::default(),
            evidence: Vec::new(),
        };
        let module = ModuleIr {
            source_file: "src/lib.rs".to_owned(),
            language: "rust".to_owned(),
            source_digest: hex_sha256(b"source"),
            graph_node_id: None,
            functions: Vec::new(),
            coverage: Default::default(),
            evidence: Vec::new(),
        };
        let hydrated = hydrate_indexed_module(
            IndexedProgramModule {
                module,
                function_ids: vec!["second".to_owned(), "first".to_owned()],
            },
            |symbol_id| Ok(function(symbol_id)),
        )?;
        assert_eq!(
            hydrated
                .functions
                .iter()
                .map(|function| function.symbol_id.as_str())
                .collect::<Vec<_>>(),
            ["second", "first"]
        );
        Ok(())
    }
}
