use std::collections::{BTreeMap, BTreeSet};

use compass_model::code_graph::NodeRecord;

#[allow(dead_code)]
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub(crate) enum CandidateSource {
    ExactId,
    ExactName,
    Alias,
    TermIndex,
    RelationSeed,
    Fuzzy,
    HeuristicFallback,
}

impl CandidateSource {
    #[must_use]
    pub(crate) const fn priority(self) -> u8 {
        match self {
            Self::ExactId => 6,
            Self::ExactName => 5,
            Self::Alias => 4,
            Self::TermIndex => 3,
            Self::RelationSeed => 2,
            Self::Fuzzy => 1,
            Self::HeuristicFallback => 0,
        }
    }
}

#[derive(Clone, Debug)]
pub(crate) struct SearchCandidate {
    pub(crate) node: NodeRecord,
    pub(crate) sources: BTreeSet<CandidateSource>,
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct RecallBudget {
    pub(crate) max_total_candidates: usize,
    pub(crate) max_per_source: usize,
}

#[derive(Debug)]
pub(crate) struct SearchCandidatePool {
    candidates: BTreeMap<String, SearchCandidate>,
    source_counts: BTreeMap<CandidateSource, usize>,
    budget: RecallBudget,
    pub(crate) truncated: bool,
}

impl SearchCandidatePool {
    pub(crate) const fn new(budget: RecallBudget) -> Self {
        Self {
            candidates: BTreeMap::new(),
            source_counts: BTreeMap::new(),
            budget,
            truncated: false,
        }
    }

    fn has_capacity_for_source(&self, source: CandidateSource) -> bool {
        self.source_counts
            .get(&source)
            .is_none_or(|count| *count < self.budget.max_per_source)
    }

    pub(crate) fn add(&mut self, source: CandidateSource, node: NodeRecord) {
        if let Some(record) = self.candidates.get_mut(&node.id) {
            record.sources.insert(source);
            return;
        }
        if self.candidates.len() >= self.budget.max_total_candidates {
            self.truncated = true;
            return;
        }
        if !self.has_capacity_for_source(source) {
            self.truncated = true;
            return;
        }
        self.source_counts
            .entry(source)
            .and_modify(|count| *count += 1)
            .or_insert(1);
        self.candidates.insert(
            node.id.clone(),
            SearchCandidate {
                node,
                sources: BTreeSet::from([source]),
            },
        );
    }

    pub(crate) fn add_many<I>(&mut self, source: CandidateSource, nodes: I)
    where
        I: IntoIterator<Item = NodeRecord>,
    {
        for node in nodes {
            self.add(source, node);
            if self.candidates.len() >= self.budget.max_total_candidates
                && self.has_capacity_for_source(source)
            {
                self.truncated = true;
            }
        }
    }

    #[must_use]
    pub(crate) fn into_vec(self) -> Vec<SearchCandidate> {
        self.candidates.into_values().collect()
    }
}

impl SearchCandidate {
    #[must_use]
    pub(crate) fn best_source_rank(&self) -> u8 {
        self.sources
            .iter()
            .map(|source| source.priority())
            .max()
            .unwrap_or(0)
    }
}
