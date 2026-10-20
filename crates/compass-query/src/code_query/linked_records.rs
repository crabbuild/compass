//! Record access for the bounded witnessed-call profile. JSON borrows pinned
//! records; store decoding is separately capped at256KiB per record. Full
//! metadata is preflighted before proof validation or response projection.
use super::*;
use crate::record_budget::RecordBudget;
use std::borrow::Cow;

type CallRead<'a> = (Vec<Cow<'a, EdgeRecord>>, bool, usize);

fn record_limits(items: usize) -> Result<SnapshotReadLimits, QueryError> {
    let mut limits = snapshot_limits(items)?;
    // Existing Merkle values are at most256KiB. Include bounded encoded keys,
    // and do not inherit the general1GiB hydration allowance for this channel.
    limits.max_bytes = items.saturating_mul(compass_store::MAX_VALUE_BYTES + 2048);
    limits.max_objects = items.saturating_mul(64).saturating_add(1);
    Ok(limits)
}

fn linked_read_error(error: compass_graph::SnapshotError) -> QueryError {
    match error {
        compass_graph::SnapshotError::Limit(message) => QueryError::new(
            QueryErrorKind::ExpansionLimit,
            "linked_record_read_limit",
            format!(
                "Witnessed record read reached its bounded store allowance: {message}; narrow the question or scope"
            ),
        ),
        other => snapshot_error(other),
    }
}

impl PinnedDiscoveryBackend<'_> {
    pub(crate) fn node_for_link(
        &self,
        id: &str,
        budget: &mut RecordBudget,
    ) -> Result<Option<Cow<'_, NodeRecord>>, QueryError> {
        RecordBudget::identity(id)?;
        match self {
            Self::Materialized { graph, lookup, .. } => {
                let Some(index) = lookup.node_by_id(id) else {
                    return Ok(None);
                };
                let node = &graph.nodes[index];
                budget.node(node)?;
                Ok(Some(Cow::Borrowed(node)))
            }
            Self::Store(reader) => {
                // get_node uses a bounded immutable256KiB store value. Its
                // owned decode is moved, with no second full-record clone.
                let Some(node) = reader.get_node(id).map_err(linked_read_error)? else {
                    return Ok(None);
                };
                budget.node(&node)?;
                Ok(Some(Cow::Owned(node)))
            }
        }
    }

    pub(crate) fn calls_for_link(
        &self,
        id: &str,
        limit: usize,
        budget: &mut RecordBudget,
    ) -> Result<CallRead<'_>, QueryError> {
        RecordBudget::identity(id)?;
        if limit > 16 {
            return Err(QueryError::new(
                QueryErrorKind::ExpansionLimit,
                "linked_fanout_limit",
                "Witnessed call context fanout is limited to16 retained occurrences",
            ));
        }
        match self {
            Self::Materialized {
                graph, adjacency, ..
            } => {
                let (indices, truncated, _) =
                    adjacency.matching_bounded(graph, id, false, &[EdgeKind::Calls], true, limit);
                let examined = indices.len() + usize::from(truncated);
                let mut records = Vec::with_capacity(indices.len());
                for index in indices {
                    let edge = &graph.links[index];
                    budget.edge(edge)?;
                    if !is_heuristic(edge) {
                        records.push(Cow::Borrowed(edge));
                    }
                }
                Ok((records, truncated, examined))
            }
            Self::Store(reader) => {
                // Read the bounded ID prefix before hydrating payloads; the
                // lookahead occurrence is counted but its record is not copied.
                let mut limits = snapshot_limits(limit.max(1))?;
                limits.max_bytes = 64 * 1024;
                limits.max_objects = 1024;
                let (mut ids, limited) = reader
                    .adjacency_edge_ids_by_kinds(id, false, &[EdgeKind::Calls], limits)
                    .map_err(linked_read_error)?;
                let truncated = limited || ids.len() > limit;
                ids.truncate(limit);
                let examined = ids.len() + usize::from(truncated);
                if ids.is_empty() {
                    return Ok((Vec::new(), truncated, examined));
                }
                for id in &ids {
                    RecordBudget::identity(id)?;
                }
                let ids = ids.into_iter().collect::<BTreeSet<_>>();
                let edges = reader
                    .get_edges_by_ids_bounded_work(&ids, record_limits(ids.len())?)
                    .map_err(linked_read_error)?;
                let mut records = Vec::with_capacity(edges.len());
                for edge in edges {
                    budget.edge(&edge)?;
                    if !is_heuristic(&edge) {
                        records.push(Cow::Owned(edge));
                    }
                }
                Ok((records, truncated, examined))
            }
        }
    }

    pub(crate) fn incident_for_link(
        &self,
        id: &str,
        include_heuristic: bool,
        limit: usize,
        budget: &mut RecordBudget,
    ) -> Result<CallRead<'_>, QueryError> {
        RecordBudget::identity(id)?;
        if limit > compass_model::query_contract::MAX_DISCOVERY_EDGES as usize {
            return Err(QueryError::new(
                QueryErrorKind::ExpansionLimit,
                "linked_incident_limit",
                "Incident context exceeds the discovery edge limit",
            ));
        }
        match self {
            Self::Materialized {
                graph, adjacency, ..
            } => {
                let prefix = adjacency.incident(id, true);
                let truncated = prefix.len() > limit;
                let mut records = Vec::with_capacity(prefix.len().min(limit));
                for index in prefix.iter().take(limit) {
                    let edge = &graph.links[*index];
                    budget.edge(edge)?;
                    if include_heuristic || !is_heuristic(edge) {
                        records.push(Cow::Borrowed(edge));
                    }
                }
                Ok((
                    records,
                    truncated,
                    prefix.len().min(limit) + usize::from(truncated),
                ))
            }
            Self::Store(reader) => {
                let mut limits = snapshot_limits(limit.saturating_add(1))?;
                limits.max_bytes = 64 * 1024;
                limits.max_objects = 1024;
                let (mut ids, limited) = reader
                    .incident_edge_ids(id, limits)
                    .map_err(linked_read_error)?;
                let truncated = limited || ids.len() > limit;
                ids.truncate(limit);
                let examined = ids.len() + usize::from(truncated);
                let mut records = Vec::with_capacity(ids.len());
                // Hydrate one portable record at a time. The aggregate budget
                // is checked before retaining it or decoding the next payload.
                for id in ids {
                    RecordBudget::identity(&id)?;
                    let edge = reader
                        .get_edge(&id)
                        .map_err(linked_read_error)?
                        .ok_or_else(|| {
                            QueryError::new(
                                QueryErrorKind::GraphInvariant,
                                "linked_edge_missing",
                                "A pinned incident occurrence is missing",
                            )
                        })?;
                    budget.edge(&edge)?;
                    if include_heuristic || !is_heuristic(&edge) {
                        records.push(Cow::Owned(edge));
                    }
                }
                Ok((records, truncated, examined))
            }
        }
    }

    pub(crate) fn edges_for_link(
        &self,
        ids: &BTreeSet<String>,
        budget: &mut RecordBudget,
    ) -> Result<Vec<Cow<'_, EdgeRecord>>, QueryError> {
        if ids.len() > 8 {
            return Err(QueryError::new(
                QueryErrorKind::ExpansionLimit,
                "linked_path_edge_limit",
                "Two four-hop witnesses retain at most8 distinct occurrences",
            ));
        }
        if ids.is_empty() {
            return Ok(Vec::new());
        }
        for id in ids {
            RecordBudget::identity(id)?;
        }
        match self {
            Self::Materialized {
                graph, adjacency, ..
            } => ids
                .iter()
                .map(|id| {
                    let index = adjacency.by_id(id).ok_or_else(|| {
                        QueryError::new(
                            QueryErrorKind::GraphInvariant,
                            "linked_edge_missing",
                            "A pinned selected call occurrence is missing",
                        )
                    })?;
                    let edge = &graph.links[index];
                    budget.edge(edge)?;
                    Ok(Cow::Borrowed(edge))
                })
                .collect(),
            Self::Store(reader) => reader
                .get_edges_by_ids_bounded_work(ids, record_limits(ids.len())?)
                .map_err(linked_read_error)?
                .into_iter()
                .map(|edge| {
                    budget.edge(&edge)?;
                    Ok(Cow::Owned(edge))
                })
                .collect(),
        }
    }
}

#[cfg(test)]
use crate::test_support;

#[cfg(test)]
mod tests {
    use super::*;
    use std::error::Error;

    #[test]
    fn materialized_reads_borrow_records_and_store_reads_move_bounded_decodes()
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
        let prepared = compass_graph::GraphSnapshotBuilder::new().prepare(&store, &graph)?;
        compass_graph::GraphSnapshotBuilder::new().activate(&store, &prepared)?;
        let stored = PinnedDiscoveryBackend::Store(Box::new(
            GraphSnapshotReader::open_active(&store)?.ok_or("snapshot missing")?,
        ));
        let mut budget = RecordBudget::new();
        let borrowed = json
            .node_for_link("n:list", &mut budget)?
            .ok_or("borrowed node")?;
        assert!(matches!(borrowed, Cow::Borrowed(_)));
        assert!(std::ptr::eq(borrowed.as_ref(), &graph.nodes[0]));
        let mut budget = RecordBudget::new();
        let owned = stored
            .node_for_link("n:list", &mut budget)?
            .ok_or("stored node")?;
        assert!(matches!(owned, Cow::Owned(_)));
        assert_eq!(borrowed.as_ref(), owned.as_ref());
        for backend in [&json, &stored] {
            let mut budget = RecordBudget::new();
            budget.projection(&"x".repeat(crate::record_budget::MAX_TEXT_BYTES - 1))?;
            let error = backend
                .node_for_link("n:list", &mut budget)
                .err()
                .ok_or("exhausted metadata budget returned a record")?;
            assert_eq!(error.code(), "linked_record_metadata_limit");
        }
        Ok(())
    }

    #[test]
    fn call_id_prefix_has_backend_parity_and_counts_filtered_lookahead()
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
        let prepared = compass_graph::GraphSnapshotBuilder::new().prepare(&store, &graph)?;
        compass_graph::GraphSnapshotBuilder::new().activate(&store, &prepared)?;
        let stored = PinnedDiscoveryBackend::Store(Box::new(
            GraphSnapshotReader::open_active(&store)?.ok_or("snapshot missing")?,
        ));
        for id in ["n:caller", "n:heuristic", "missing"] {
            for limit in [0, 1, 8, 16] {
                let mut budget = RecordBudget::new();
                let (left, limited, examined) = json.calls_for_link(id, limit, &mut budget)?;
                let mut budget = RecordBudget::new();
                let (right, right_limited, right_examined) =
                    stored.calls_for_link(id, limit, &mut budget)?;
                assert_eq!(limited, right_limited, "{id}, {limit}");
                assert_eq!(examined, right_examined, "{id}, {limit}");
                assert_eq!(
                    left.iter().map(|edge| edge.as_ref()).collect::<Vec<_>>(),
                    right.iter().map(|edge| edge.as_ref()).collect::<Vec<_>>()
                );
                assert!(left.iter().all(|edge| matches!(edge, Cow::Borrowed(_))));
                assert!(right.iter().all(|edge| matches!(edge, Cow::Owned(_))));
                assert!(examined <= limit + 1);
            }
            for limit in [0, 1, 16, 1000] {
                for include_heuristic in [false, true] {
                    let mut budget = RecordBudget::new();
                    let (left, limited, examined) =
                        json.incident_for_link(id, include_heuristic, limit, &mut budget)?;
                    let mut budget = RecordBudget::new();
                    let (right, right_limited, right_examined) =
                        stored.incident_for_link(id, include_heuristic, limit, &mut budget)?;
                    assert_eq!((limited, examined), (right_limited, right_examined));
                    assert_eq!(
                        left.iter().map(|edge| edge.as_ref()).collect::<Vec<_>>(),
                        right.iter().map(|edge| edge.as_ref()).collect::<Vec<_>>()
                    );
                    assert!(examined <= limit + 1);
                    assert!(left.iter().all(|edge| matches!(edge, Cow::Borrowed(_))));
                    assert!(right.iter().all(|edge| matches!(edge, Cow::Owned(_))));
                }
            }
        }
        for backend in [&json, &stored] {
            let mut budget = RecordBudget::new();
            budget.projection(&"x".repeat(crate::record_budget::MAX_TEXT_BYTES - 1))?;
            let error = backend
                .incident_for_link("n:caller", false, 16, &mut budget)
                .err()
                .ok_or("exhausted incident metadata budget returned records")?;
            assert_eq!(error.code(), "linked_record_metadata_limit");
        }
        assert_eq!(
            linked_read_error(compass_graph::SnapshotError::Limit("test".to_owned())).kind(),
            QueryErrorKind::ExpansionLimit
        );
        Ok(())
    }
}
