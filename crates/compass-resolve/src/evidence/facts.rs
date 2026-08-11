//! Validated primary facts consumed by immutable resolution indexes.

use super::*;

pub(in crate::evidence) trait FactId {
    fn fact_id(&self) -> &str;
}

macro_rules! impl_fact_id {
    ($($fact:ty),+ $(,)?) => {
        $(
            impl FactId for $fact {
                fn fact_id(&self) -> &str {
                    &self.id
                }
            }
        )+
    };
}

impl_fact_id!(
    DeclarationFact,
    OccurrenceFact,
    compass_languages::BindingFact,
    RelationshipCandidate,
    compass_languages::ScopeFact,
);

pub(in crate::evidence) struct FactTable<T> {
    values: Vec<T>,
}

impl<T: FactId> FactTable<T> {
    pub(in crate::evidence) fn from_values(mut values: Vec<T>) -> Result<Self, String> {
        values.sort_unstable_by(|left, right| left.fact_id().cmp(right.fact_id()));
        if let Some(duplicate) = values
            .windows(2)
            .find(|pair| pair[0].fact_id() == pair[1].fact_id())
        {
            return Err(format!(
                "duplicate universal evidence id {:?}",
                duplicate[0].fact_id()
            ));
        }
        values.shrink_to_fit();
        Ok(Self { values })
    }

    pub(in crate::evidence) fn get(&self, id: &str) -> Option<&T> {
        self.values
            .binary_search_by(|fact| fact.fact_id().cmp(id))
            .ok()
            .map(|index| &self.values[index])
    }

    pub(in crate::evidence) fn values(&self) -> std::slice::Iter<'_, T> {
        self.values.iter()
    }

    pub(in crate::evidence) const fn len(&self) -> usize {
        self.values.len()
    }
}

pub(in crate::evidence) type CandidateSlot = u32;
type StringSlot = u32;

struct CompactCandidate {
    id: StringSlot,
    language: StringSlot,
    relation: CandidateRelation,
    source_declaration_id: StringSlot,
    occurrence_id: Option<StringSlot>,
    binding_id: Option<StringSlot>,
    target_spelling: StringSlot,
    constraints: CompactConstraint,
}

struct CompactConstraint {
    exact_target_declaration_id: Option<StringSlot>,
    exact_language: Option<StringSlot>,
    module_or_package: Option<StringSlot>,
    scope_id: Option<StringSlot>,
    qualified_name: Option<StringSlot>,
    argument_count: Option<u32>,
    argument_types: Vec<Option<StringSlot>>,
    allowed_target_kinds: Vec<StringSlot>,
    hierarchy: Option<CompactHierarchy>,
    allow_external: bool,
}

enum CompactHierarchy {
    DirectBase {
        base_set_complete: bool,
    },
    ReceiverDispatch {
        receiver_qualified_name: StringSlot,
        strategy: ReceiverDispatchStrategy,
    },
    RustAssociatedType {
        receiver_declaration_id: StringSlot,
        receiver_qualified_name: StringSlot,
        trait_qualified_name: StringSlot,
    },
}

pub(in crate::evidence) struct CandidateTableBuilder {
    strings: StringPoolBuilder,
    values: Vec<CompactCandidate>,
}

impl CandidateTableBuilder {
    pub(in crate::evidence) fn with_capacity(capacity: usize) -> Self {
        Self {
            strings: StringPoolBuilder::default(),
            values: Vec::with_capacity(capacity),
        }
    }

    pub(in crate::evidence) fn extend(
        &mut self,
        values: Vec<RelationshipCandidate>,
    ) -> Result<(), String> {
        for candidate in values {
            self.values
                .push(CompactCandidate::new(candidate, &mut self.strings)?);
        }
        Ok(())
    }

    pub(in crate::evidence) fn finish(self) -> Result<CandidateTable, String> {
        if u32::try_from(self.values.len()).is_err() {
            return Err("universal candidate slot count exceeds u32".to_owned());
        }
        let strings = self.strings.finish()?;
        let mut values = self.values;
        values.sort_unstable_by(|left, right| {
            strings[left.id as usize].cmp(&strings[right.id as usize])
        });
        if let Some(duplicate) = values
            .windows(2)
            .find(|pair| strings[pair[0].id as usize] == strings[pair[1].id as usize])
        {
            return Err(format!(
                "duplicate universal evidence id {:?}",
                strings[duplicate[0].id as usize]
            ));
        }
        values.shrink_to_fit();
        Ok(CandidateTable { strings, values })
    }
}

pub(in crate::evidence) struct CandidateTable {
    strings: Vec<String>,
    values: Vec<CompactCandidate>,
}

impl CandidateTable {
    #[cfg(test)]
    pub(in crate::evidence) fn from_values(
        values: Vec<RelationshipCandidate>,
    ) -> Result<Self, String> {
        let mut builder = CandidateTableBuilder::with_capacity(values.len());
        builder.extend(values)?;
        builder.finish()
    }

    pub(in crate::evidence) fn get(&self, id: &str) -> Option<RelationshipCandidate> {
        let index = self
            .values
            .binary_search_by(|candidate| self.string(candidate.id).cmp(id))
            .ok()?;
        self.at(u32::try_from(index).ok()?)
    }

    pub(in crate::evidence) fn at(&self, slot: CandidateSlot) -> Option<RelationshipCandidate> {
        self.values
            .get(usize::try_from(slot).ok()?)
            .map(|candidate| candidate.inflate(self))
    }

    pub(in crate::evidence) fn values(&self) -> impl Iterator<Item = RelationshipCandidate> + '_ {
        self.values.iter().map(|candidate| candidate.inflate(self))
    }

    pub(in crate::evidence) fn slots(&self) -> impl Iterator<Item = CandidateSlot> + '_ {
        (0..self.values.len()).filter_map(|index| u32::try_from(index).ok())
    }

    pub(in crate::evidence) fn id(&self, slot: CandidateSlot) -> Option<&str> {
        let candidate = self.values.get(usize::try_from(slot).ok()?)?;
        Some(self.string(candidate.id))
    }

    pub(in crate::evidence) fn occurrence_id(&self, slot: CandidateSlot) -> Option<&str> {
        let candidate = self.values.get(usize::try_from(slot).ok()?)?;
        candidate.occurrence_id.map(|id| self.string(id))
    }

    pub(in crate::evidence) fn source_declaration_id(&self, slot: CandidateSlot) -> Option<&str> {
        let candidate = self.values.get(usize::try_from(slot).ok()?)?;
        Some(self.string(candidate.source_declaration_id))
    }

    pub(in crate::evidence) const fn len(&self) -> usize {
        self.values.len()
    }

    fn string(&self, slot: StringSlot) -> &str {
        &self.strings[slot as usize]
    }
}

impl CompactCandidate {
    fn new(
        candidate: RelationshipCandidate,
        strings: &mut StringPoolBuilder,
    ) -> Result<Self, String> {
        Ok(Self {
            id: strings.intern(candidate.id)?,
            language: strings.intern(candidate.language)?,
            relation: candidate.relation,
            source_declaration_id: strings.intern(candidate.source_declaration_id)?,
            occurrence_id: strings.intern_option(candidate.occurrence_id)?,
            binding_id: strings.intern_option(candidate.binding_id)?,
            target_spelling: strings.intern(candidate.target_spelling)?,
            constraints: CompactConstraint::new(candidate.constraints, strings)?,
        })
    }

    fn inflate(&self, table: &CandidateTable) -> RelationshipCandidate {
        RelationshipCandidate {
            id: table.string(self.id).to_owned(),
            language: table.string(self.language).to_owned(),
            relation: self.relation,
            source_declaration_id: table.string(self.source_declaration_id).to_owned(),
            occurrence_id: self.occurrence_id.map(|slot| table.string(slot).to_owned()),
            binding_id: self.binding_id.map(|slot| table.string(slot).to_owned()),
            target_spelling: table.string(self.target_spelling).to_owned(),
            constraints: self.constraints.inflate(table),
        }
    }
}

impl CompactConstraint {
    fn new(
        constraint: compass_languages::ResolutionConstraint,
        strings: &mut StringPoolBuilder,
    ) -> Result<Self, String> {
        Ok(Self {
            exact_target_declaration_id: strings
                .intern_option(constraint.exact_target_declaration_id)?,
            exact_language: strings.intern_option(constraint.exact_language)?,
            module_or_package: strings.intern_option(constraint.module_or_package)?,
            scope_id: strings.intern_option(constraint.scope_id)?,
            qualified_name: strings.intern_option(constraint.qualified_name)?,
            argument_count: constraint.argument_count,
            argument_types: constraint
                .argument_types
                .into_iter()
                .map(|value| strings.intern_option(value))
                .collect::<Result<Vec<_>, _>>()?,
            allowed_target_kinds: constraint
                .allowed_target_kinds
                .into_iter()
                .map(|value| strings.intern(value))
                .collect::<Result<Vec<_>, _>>()?,
            hierarchy: constraint
                .hierarchy
                .map(|hierarchy| CompactHierarchy::new(hierarchy, strings))
                .transpose()?,
            allow_external: constraint.allow_external,
        })
    }

    fn inflate(&self, table: &CandidateTable) -> compass_languages::ResolutionConstraint {
        compass_languages::ResolutionConstraint {
            exact_target_declaration_id: self
                .exact_target_declaration_id
                .map(|slot| table.string(slot).to_owned()),
            exact_language: self
                .exact_language
                .map(|slot| table.string(slot).to_owned()),
            module_or_package: self
                .module_or_package
                .map(|slot| table.string(slot).to_owned()),
            scope_id: self.scope_id.map(|slot| table.string(slot).to_owned()),
            qualified_name: self
                .qualified_name
                .map(|slot| table.string(slot).to_owned()),
            argument_count: self.argument_count,
            argument_types: self
                .argument_types
                .iter()
                .map(|value| value.map(|slot| table.string(slot).to_owned()))
                .collect(),
            allowed_target_kinds: self
                .allowed_target_kinds
                .iter()
                .map(|slot| table.string(*slot).to_owned())
                .collect(),
            hierarchy: self
                .hierarchy
                .as_ref()
                .map(|hierarchy| hierarchy.inflate(table)),
            allow_external: self.allow_external,
        }
    }
}

impl CompactHierarchy {
    fn new(
        hierarchy: HierarchyConstraint,
        strings: &mut StringPoolBuilder,
    ) -> Result<Self, String> {
        Ok(match hierarchy {
            HierarchyConstraint::DirectBase { base_set_complete } => {
                Self::DirectBase { base_set_complete }
            }
            HierarchyConstraint::ReceiverDispatch {
                receiver_qualified_name,
                strategy,
            } => Self::ReceiverDispatch {
                receiver_qualified_name: strings.intern(receiver_qualified_name)?,
                strategy,
            },
            HierarchyConstraint::RustAssociatedType {
                receiver_declaration_id,
                receiver_qualified_name,
                trait_qualified_name,
            } => Self::RustAssociatedType {
                receiver_declaration_id: strings.intern(receiver_declaration_id)?,
                receiver_qualified_name: strings.intern(receiver_qualified_name)?,
                trait_qualified_name: strings.intern(trait_qualified_name)?,
            },
        })
    }

    fn inflate(&self, table: &CandidateTable) -> HierarchyConstraint {
        match self {
            Self::DirectBase { base_set_complete } => HierarchyConstraint::DirectBase {
                base_set_complete: *base_set_complete,
            },
            Self::ReceiverDispatch {
                receiver_qualified_name,
                strategy,
            } => HierarchyConstraint::ReceiverDispatch {
                receiver_qualified_name: table.string(*receiver_qualified_name).to_owned(),
                strategy: *strategy,
            },
            Self::RustAssociatedType {
                receiver_declaration_id,
                receiver_qualified_name,
                trait_qualified_name,
            } => HierarchyConstraint::RustAssociatedType {
                receiver_declaration_id: table.string(*receiver_declaration_id).to_owned(),
                receiver_qualified_name: table.string(*receiver_qualified_name).to_owned(),
                trait_qualified_name: table.string(*trait_qualified_name).to_owned(),
            },
        }
    }
}

#[derive(Default)]
struct StringPoolBuilder {
    slots: AHashMap<String, StringSlot>,
}

impl StringPoolBuilder {
    fn intern(&mut self, value: String) -> Result<StringSlot, String> {
        if let Some(slot) = self.slots.get(&value) {
            return Ok(*slot);
        }
        let slot = u32::try_from(self.slots.len())
            .map_err(|_| "universal candidate string slot count exceeds u32".to_owned())?;
        self.slots.insert(value, slot);
        Ok(slot)
    }

    fn intern_option(&mut self, value: Option<String>) -> Result<Option<StringSlot>, String> {
        value.map(|value| self.intern(value)).transpose()
    }

    fn finish(self) -> Result<Vec<String>, String> {
        let mut ordered = std::iter::repeat_with(|| None)
            .take(self.slots.len())
            .collect::<Vec<_>>();
        for (value, slot) in self.slots {
            let Some(destination) = ordered.get_mut(slot as usize) else {
                return Err("invalid universal candidate string slot".to_owned());
            };
            *destination = Some(value);
        }
        ordered
            .into_iter()
            .map(|value| value.ok_or_else(|| "missing universal candidate string slot".to_owned()))
            .collect()
    }
}

pub(super) struct FactStore {
    pub(super) declarations: FactTable<DeclarationFact>,
    pub(super) declaration_ids: Vec<String>,
    pub(super) occurrences: FactTable<OccurrenceFact>,
    pub(super) bindings: FactTable<compass_languages::BindingFact>,
    pub(super) candidates: CandidateTable,
    pub(super) scopes: FactTable<compass_languages::ScopeFact>,
    pub(super) definition_ranges: BTreeMap<String, EvidenceRange>,
}

#[cfg(test)]
mod tests {
    use compass_languages::{
        CandidateRelation, HierarchyConstraint, ReceiverDispatchStrategy, RelationshipCandidate,
        ResolutionConstraint,
    };

    use super::{CandidateTable, FactId, FactTable};

    struct TestFact {
        id: String,
    }

    impl FactId for TestFact {
        fn fact_id(&self) -> &str {
            &self.id
        }
    }

    #[test]
    fn fact_table_sorts_for_borrowed_lookup_and_rejects_duplicate_ids() -> Result<(), String> {
        let table = FactTable::from_values(vec![fact("z"), fact("a")])?;
        assert_eq!(table.get("a").map(FactId::fact_id), Some("a"));
        assert_eq!(table.get("missing").map(FactId::fact_id), None);
        assert!(
            FactTable::from_values(vec![fact("same"), fact("same")])
                .is_err_and(|error| error.contains("duplicate universal evidence id"))
        );
        Ok(())
    }

    fn fact(id: &str) -> TestFact {
        TestFact { id: id.to_owned() }
    }

    #[test]
    fn candidate_table_round_trips_every_compacted_field() -> Result<(), String> {
        let candidate = RelationshipCandidate {
            id: "candidate:1".to_owned(),
            language: "rust".to_owned(),
            relation: CandidateRelation::Calls,
            source_declaration_id: "declaration:source".to_owned(),
            occurrence_id: Some("occurrence:1".to_owned()),
            binding_id: Some("binding:1".to_owned()),
            target_spelling: "execute".to_owned(),
            constraints: ResolutionConstraint {
                exact_target_declaration_id: Some("declaration:target".to_owned()),
                exact_language: Some("rust".to_owned()),
                module_or_package: Some("crate::module".to_owned()),
                scope_id: Some("scope:1".to_owned()),
                qualified_name: Some("crate::module::execute".to_owned()),
                argument_count: Some(2),
                argument_types: vec![Some("Input".to_owned()), None],
                allowed_target_kinds: vec!["function".to_owned(), "method".to_owned()],
                hierarchy: Some(HierarchyConstraint::ReceiverDispatch {
                    receiver_qualified_name: "crate::Receiver".to_owned(),
                    strategy: ReceiverDispatchStrategy::C3AfterReceiver,
                }),
                allow_external: true,
            },
        };
        let table = CandidateTable::from_values(vec![candidate.clone()])?;
        assert_eq!(table.get(&candidate.id), Some(candidate.clone()));
        assert_eq!(table.at(0), Some(candidate));
        assert_eq!(table.id(0), Some("candidate:1"));
        assert_eq!(table.occurrence_id(0), Some("occurrence:1"));
        assert_eq!(table.source_declaration_id(0), Some("declaration:source"));
        assert!(
            CandidateTable::from_values(vec![
                candidate_with_id("duplicate"),
                candidate_with_id("duplicate")
            ])
            .is_err_and(|error| error.contains("duplicate universal evidence id"))
        );
        Ok(())
    }

    fn candidate_with_id(id: &str) -> RelationshipCandidate {
        RelationshipCandidate {
            id: id.to_owned(),
            language: "rust".to_owned(),
            relation: CandidateRelation::Calls,
            source_declaration_id: "source".to_owned(),
            occurrence_id: None,
            binding_id: None,
            target_spelling: "target".to_owned(),
            constraints: ResolutionConstraint::default(),
        }
    }
}
