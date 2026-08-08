use std::collections::{BTreeMap, BTreeSet};

use compass_model::code_graph::NodeRecord;

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
enum RecallTruncationReason {
    Total,
    PerSource,
    Fuzzy,
}

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
    pub(crate) max_fuzzy_candidates: usize,
}

#[derive(Debug)]
pub(crate) struct SearchCandidatePool {
    candidates: BTreeMap<String, SearchCandidate>,
    source_counts: BTreeMap<CandidateSource, usize>,
    budget: RecallBudget,
    truncation_reasons: BTreeSet<RecallTruncationReason>,
    fuzzy_limit_reached: bool,
    fuzzy_candidates_added: usize,
    candidates_read: u64,
}

impl SearchCandidatePool {
    pub(crate) const fn new(budget: RecallBudget) -> Self {
        Self {
            candidates: BTreeMap::new(),
            source_counts: BTreeMap::new(),
            budget,
            truncation_reasons: BTreeSet::new(),
            fuzzy_limit_reached: false,
            fuzzy_candidates_added: 0,
            candidates_read: 0,
        }
    }

    #[must_use]
    pub(crate) fn len(&self) -> usize {
        self.candidates.len()
    }

    #[must_use]
    pub(crate) fn is_truncated(&self) -> bool {
        !self.truncation_reasons.is_empty()
    }

    fn mark_truncated(&mut self, reason: RecallTruncationReason) {
        self.truncation_reasons.insert(reason);
    }

    fn has_capacity_for_source(&self, source: CandidateSource) -> bool {
        self.source_counts
            .get(&source)
            .is_none_or(|count| *count < self.budget.max_per_source)
    }

    pub(crate) fn add(&mut self, source: CandidateSource, node: NodeRecord) -> bool {
        self.candidates_read = self.candidates_read.saturating_add(1);
        if let Some(record) = self.candidates.get_mut(&node.id) {
            record.sources.insert(source);
            return false;
        }
        if self.candidates.len() >= self.budget.max_total_candidates {
            self.mark_truncated(RecallTruncationReason::Total);
            return false;
        }
        if !self.has_capacity_for_source(source) {
            self.mark_truncated(RecallTruncationReason::PerSource);
            return false;
        }
        if source == CandidateSource::Fuzzy
            && self.fuzzy_candidates_added >= self.budget.max_fuzzy_candidates
        {
            self.mark_truncated(RecallTruncationReason::Fuzzy);
            self.fuzzy_limit_reached = true;
            return false;
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
        if source == CandidateSource::Fuzzy {
            self.fuzzy_candidates_added = self.fuzzy_candidates_added.saturating_add(1);
        }
        true
    }

    pub(crate) fn add_many<I>(&mut self, source: CandidateSource, nodes: I)
    where
        I: IntoIterator<Item = NodeRecord>,
    {
        for node in nodes {
            self.add(source, node);
        }
    }

    #[must_use]
    pub(crate) fn into_vec(self) -> Vec<SearchCandidate> {
        self.candidates.into_values().collect()
    }

    #[must_use]
    pub(crate) fn candidates_read(&self) -> u64 {
        self.candidates_read
    }

    pub(crate) fn candidate_ids(&self) -> Vec<String> {
        self.candidates.keys().cloned().collect()
    }

    pub(crate) fn tag(&mut self, node_id: &str, source: CandidateSource) -> bool {
        let Some(candidate) = self.candidates.get_mut(node_id) else {
            return false;
        };
        candidate.sources.insert(source)
    }

    #[must_use]
    pub(crate) fn truncated_by_fuzzy_capacity(&self) -> bool {
        self.fuzzy_limit_reached
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

#[cfg(test)]
mod tests {
    use compass_model::code_graph::{NodeKind, NodeRecord};
    use compass_model::provenance::{EvidenceConfidence, EvidenceOrigin, Provenance, SourceAnchor};

    use super::{CandidateSource, RecallBudget, SearchCandidatePool};

    fn source_anchor() -> SourceAnchor {
        SourceAnchor {
            file: "src/lib.rs".to_owned(),
            start_byte: 1,
            end_byte: 2,
            start_line: 1,
            start_column: 1,
            end_line: 1,
            end_column: 1,
        }
    }

    fn node(id: &str) -> NodeRecord {
        NodeRecord {
            id: id.to_owned(),
            kind: NodeKind::Function,
            roles: Vec::new(),
            name: id.to_owned(),
            qualified_name: id.to_owned(),
            language: None,
            framework: None,
            source: Some(source_anchor()),
            details: None,
            evidence: vec![Provenance {
                origin: EvidenceOrigin::Heuristic,
                extractor: "test".to_owned(),
                confidence: EvidenceConfidence::Exact,
                rule: None,
                anchors: vec![],
                wiring_site: Some(source_anchor()),
                score: None,
                candidates: Vec::new(),
            }],
            coverage: Vec::new(),
            diagnostics: Vec::new(),
            community: None,
        }
    }

    #[test]
    fn search_candidate_pool_enforces_source_and_total_budgets() {
        let budget = RecallBudget {
            max_total_candidates: 2,
            max_per_source: 1,
            max_fuzzy_candidates: 4,
        };
        let mut pool = SearchCandidatePool::new(budget);

        assert!(pool.add(CandidateSource::ExactName, node("n:one")));
        assert!(pool.add(CandidateSource::Alias, node("n:two")));
        assert!(!pool.add(CandidateSource::Alias, node("n:three")));
        assert_eq!(pool.len(), 2);
        assert!(pool.is_truncated());
    }

    #[test]
    fn search_candidate_pool_limits_fuzzy_candidates() {
        let budget = RecallBudget {
            max_total_candidates: 8,
            max_per_source: 8,
            max_fuzzy_candidates: 1,
        };
        let mut pool = SearchCandidatePool::new(budget);

        assert!(pool.add(CandidateSource::Fuzzy, node("n:fuzzy-1")));
        assert!(!pool.add(CandidateSource::Fuzzy, node("n:fuzzy-2")));
        assert_eq!(pool.len(), 1);
        assert!(pool.truncated_by_fuzzy_capacity());
        assert!(pool.is_truncated());
    }

    #[test]
    fn search_candidate_pool_merges_duplicate_nodes() {
        let budget = RecallBudget {
            max_total_candidates: 16,
            max_per_source: 16,
            max_fuzzy_candidates: 8,
        };
        let mut pool = SearchCandidatePool::new(budget);

        let shared = node("n:shared");
        assert!(pool.add(CandidateSource::ExactName, shared.clone()));
        assert!(!pool.add(CandidateSource::Alias, shared));
        assert_eq!(pool.len(), 1);
        assert!(!pool.is_truncated());
    }
}
