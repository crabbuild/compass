//! Additional candidate-recall probes for the witnessed-call channel.
//! Indexed recall keeps its existing independent limit and reported count.
use crate::QueryError;

pub use compass_model::query_contract::{
    CALL_LINK_WORK_SCHEMA, DiscoveryCallLinkWork as RankedCallWork, MAX_CALL_LINK_CANDIDATE_PROBES,
};

pub(super) struct ProbeBudget {
    work: RankedCallWork,
}
impl ProbeBudget {
    pub(super) fn new(indexed_candidate_probes: u64) -> Self {
        Self {
            work: RankedCallWork {
                schema: CALL_LINK_WORK_SCHEMA.to_owned(),
                candidate_probes: 0,
                candidate_probe_limit: MAX_CALL_LINK_CANDIDATE_PROBES,
                indexed_candidate_probes,
                indexed_candidate_probe_limit:
                    compass_model::query_contract::MAX_INDEXED_CANDIDATE_PROBES,
            },
        }
    }
    pub(super) fn exhausted(&self) -> bool {
        self.work.candidate_probes >= self.work.candidate_probe_limit
    }
    /// Reserve exactly one probe before invoking the actual backend lookup.
    /// None means a bound was hit; the caller must mark its result bounded.
    pub(super) fn run<T>(
        &mut self,
        lookup: impl FnOnce() -> Result<T, QueryError>,
    ) -> Result<Option<T>, QueryError> {
        if self.exhausted() {
            return Ok(None);
        }
        self.work.candidate_probes += 1;
        lookup().map(Some)
    }
    pub(super) fn finish(self) -> RankedCallWork {
        self.work
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::QueryErrorKind;
    use std::cell::Cell;
    use std::error::Error;

    #[test]
    fn exact_probe_budget_counts_empty_lookups_and_refuses_plus_one_without_calling_backend()
    -> Result<(), Box<dyn Error>> {
        let calls = Cell::new(0_u64);
        let mut budget =
            ProbeBudget::new(compass_model::query_contract::MAX_INDEXED_CANDIDATE_PROBES);
        for index in 0..MAX_CALL_LINK_CANDIDATE_PROBES {
            let result = budget.run(|| {
                calls.set(calls.get() + 1);
                Ok(if index % 2 == 0 {
                    Vec::<usize>::new()
                } else {
                    vec![index as usize]
                })
            })?;
            assert!(result.is_some());
            assert_eq!(calls.get(), index + 1);
        }
        assert!(budget.exhausted());
        assert!(
            budget
                .run(|| {
                    calls.set(calls.get() + 1);
                    Ok(())
                })?
                .is_none()
        );
        assert_eq!(calls.get(), MAX_CALL_LINK_CANDIDATE_PROBES);
        let work = budget.finish();
        assert!(work.is_coherent());
        assert_eq!(work.candidate_probes, calls.get());
        assert_eq!(
            work.indexed_candidate_probes,
            compass_model::query_contract::MAX_INDEXED_CANDIDATE_PROBES
        );
        let decoded: RankedCallWork = serde_json::from_slice(&serde_json::to_vec(&work)?)?;
        assert_eq!(decoded, work);
        Ok(())
    }

    #[test]
    fn failed_backend_lookup_is_charged_before_error_propagates() {
        let calls = Cell::new(0_u64);
        let mut budget = ProbeBudget::new(267);
        let error = budget.run::<()>(|| {
            calls.set(calls.get() + 1);
            Err(QueryError::new(
                QueryErrorKind::GraphInvariant,
                "authored_lookup_error",
                "authored error",
            ))
        });
        assert!(matches!(error, Err(ref error) if error.code() == "authored_lookup_error"));
        assert_eq!(calls.get(), 1);
        assert_eq!(budget.finish().candidate_probes, 1);
    }

    #[test]
    fn work_contract_rejects_unknown_profile_and_changed_limits_or_overcounts() {
        let valid = ProbeBudget::new(291).finish();
        assert!(valid.is_coherent());
        for which in 0..5 {
            let mut forged = valid.clone();
            match which {
                0 => forged.schema = "compass.query.call-link-work/2".to_owned(),
                1 => forged.candidate_probe_limit += 1,
                2 => forged.candidate_probes = forged.candidate_probe_limit + 1,
                3 => forged.indexed_candidate_probe_limit += 1,
                _ => forged.indexed_candidate_probes = forged.indexed_candidate_probe_limit + 1,
            }
            assert!(!forged.is_coherent(), "forged work field {which}");
        }
    }
}

#[cfg(test)]
use crate::test_support;

#[cfg(test)]
mod backend_tests {
    use super::*;
    use crate::code_query::{CodeAdjacencyIndex, CodeLookupIndex, PinnedDiscoveryBackend};
    use crate::record_budget::RecordBudget;
    use compass_graph::{GraphSnapshotBuilder, GraphSnapshotReader};
    use compass_model::code_graph::GraphDocument;
    use compass_store::SqliteStore;
    use std::cell::Cell;
    use std::error::Error;

    #[test]
    fn probe_gate_wraps_actual_json_and_store_lookups_including_filtered_and_empty()
    -> Result<(), Box<dyn Error>> {
        let directory = tempfile::tempdir()?;
        let path = directory.path().join("graph.json");
        test_support::write_graph(&path)?;
        let graph = GraphDocument::load(&path)?;
        let lookup = CodeLookupIndex::build(&graph);
        let adjacency = CodeAdjacencyIndex::build(&graph);
        let json = PinnedDiscoveryBackend::Materialized {
            graph: &graph,
            lookup: &lookup,
            adjacency: &adjacency,
        };
        let store = SqliteStore::open(directory.path().join("store.sqlite3"))?;
        let prepared = GraphSnapshotBuilder::new().prepare(&store, &graph)?;
        GraphSnapshotBuilder::new().activate(&store, &prepared)?;
        let stored = PinnedDiscoveryBackend::Store(Box::new(
            GraphSnapshotReader::open_active(&store)?.ok_or("snapshot missing")?,
        ));
        for backend in [&json, &stored] {
            let mut budget = ProbeBudget::new(267);
            let calls = Cell::new(0_u64);
            let mut metadata = RecordBudget::new();
            for index in 0..MAX_CALL_LINK_CANDIDATE_PROBES {
                let id = ["n:caller", "n:heuristic", "missing"][index as usize % 3];
                let result = budget
                    .run(|| {
                        calls.set(calls.get() + 1);
                        backend.calls_for_link(id, 8, &mut metadata)
                    })?
                    .ok_or("lookup refused before limit")?;
                if id == "missing" || id == "n:heuristic" {
                    assert!(result.0.is_empty());
                }
                assert_eq!(calls.get(), index + 1);
            }
            assert!(
                budget
                    .run(|| {
                        calls.set(calls.get() + 1);
                        backend.calls_for_link("n:caller", 8, &mut metadata)
                    })?
                    .is_none()
            );
            assert_eq!(calls.get(), 40);
            assert_eq!(budget.finish().candidate_probes, calls.get());
        }
        Ok(())
    }
}
