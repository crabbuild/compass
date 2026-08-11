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

pub(super) struct FactStore {
    pub(super) declarations: FactTable<DeclarationFact>,
    pub(super) declaration_ids: Vec<String>,
    pub(super) occurrences: FactTable<OccurrenceFact>,
    pub(super) bindings: FactTable<compass_languages::BindingFact>,
    pub(super) candidates: FactTable<RelationshipCandidate>,
    pub(super) scopes: FactTable<compass_languages::ScopeFact>,
    pub(super) definition_ranges: BTreeMap<String, EvidenceRange>,
}

#[cfg(test)]
mod tests {
    use super::{FactId, FactTable};

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
}
