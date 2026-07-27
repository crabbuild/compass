use std::cell::RefCell;
use std::collections::HashMap;

use prolly::Tree;

use crate::{
    ActivityGuard, HistoryError, HistoryRecord, HistoryRecordKey, HistoryStore, PublishedVersion,
    RealizationId,
};

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
        Ok(RealizationReader {
            store: self,
            _activity: activity,
            published,
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

    pub fn read(&self, key: HistoryRecordKey<'_>) -> Result<Option<HistoryRecord>, HistoryError> {
        let owned = OwnedHistoryRecordKey::from(key);
        if let Some(value) = self.records.borrow().get(&owned) {
            return Ok(value.clone());
        }
        let (root_name, tree, encoded_key, schema) = match &owned {
            OwnedHistoryRecordKey::Node(id) => (
                "nodes",
                self.published.version.nodes_root.to_tree(),
                crate::node_key(id),
                "compass.node",
            ),
            OwnedHistoryRecordKey::ProgramModule(source_file) => (
                "program-facts",
                self.published.version.program_facts_root.to_tree(),
                crate::artifacts::program_key("module", source_file),
                "compass.program.module",
            ),
            OwnedHistoryRecordKey::ProgramFunction(symbol_id) => (
                "program-facts",
                self.published.version.program_facts_root.to_tree(),
                crate::artifacts::program_key("function", symbol_id),
                "compass.program.function",
            ),
            OwnedHistoryRecordKey::ProgramSummary(symbol_id) => (
                "program-summaries",
                self.published.version.program_summaries_root.to_tree(),
                crate::artifacts::program_key("summary", symbol_id),
                "compass.program.summary",
            ),
            OwnedHistoryRecordKey::ReverseCallers(symbol_id) => (
                "program-summaries",
                self.published.version.program_summaries_root.to_tree(),
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
        let value = match self.store.prolly.get(&tree, &encoded_key)? {
            None => None,
            Some(bytes) => {
                if bytes.len() > crate::MAX_RECORD_VALUE_BYTES {
                    return Err(HistoryError::CorruptHistory(
                        "history record exceeds byte limit".to_owned(),
                    ));
                }
                Some(match &owned {
                    OwnedHistoryRecordKey::Node(_) => {
                        HistoryRecord::Node(crate::artifacts::decode_typed(&bytes, schema)?)
                    }
                    OwnedHistoryRecordKey::ProgramModule(_) => HistoryRecord::ProgramModule(
                        crate::artifacts::decode_typed(&bytes, schema)?,
                    ),
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
}
