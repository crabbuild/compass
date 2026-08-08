//! Validated primary facts consumed by immutable resolution indexes.

use super::*;

pub(super) struct FactStore {
    pub(super) declarations: AHashMap<String, DeclarationFact>,
    pub(super) declaration_ids: Vec<String>,
    pub(super) occurrences: AHashMap<String, OccurrenceFact>,
    pub(super) bindings: AHashMap<String, compass_languages::BindingFact>,
    pub(super) candidates: AHashMap<String, RelationshipCandidate>,
    pub(super) scopes: AHashMap<String, compass_languages::ScopeFact>,
    pub(super) definition_ranges: BTreeMap<String, EvidenceRange>,
}
