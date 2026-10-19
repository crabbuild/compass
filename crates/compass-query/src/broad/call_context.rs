//! Bounded continuation of witnessed calls, never candidate admission/ranking.
use super::*;
use compass_model::query_contract::{DiscoveryEdge, DiscoveryTraversal};

const MAX_EXPANSIONS: usize = 32;
const MAX_FANOUT: usize = 16;
const MAX_FRONTIER: usize = 64;

pub(super) struct Context<'a> {
    pub scopes: &'a [DiscoveryScope],
    pub nodes: &'a mut BTreeMap<String, QueryNode>,
    pub edges: &'a mut BTreeMap<String, DiscoveryEdge>,
    pub seeds: &'a [DiscoverySeed],
    pub expanded: &'a mut u64,
    pub metadata: Option<&'a mut crate::record_budget::RecordBudget>,
}

impl Context<'_> {
    pub(super) fn extend(
        self,
        backend: &PinnedDiscoveryBackend<'_>,
        request: &DiscoveryQueryRequest,
        check: &dyn Fn() -> Result<(), QueryError>,
    ) -> Result<bool, QueryError> {
        let Self {
            scopes,
            nodes,
            edges,
            seeds,
            expanded,
            mut metadata,
        } = self;
        if request.limits.max_depth <= 1 {
            return Ok(false);
        }
        let mut frontier = seeds
            .iter()
            .map(|seed| (seed.node_id.clone(), 0_u32))
            .collect::<VecDeque<_>>();
        let mut depths = frontier.iter().cloned().collect::<BTreeMap<_, _>>();
        let mut expansions = 0;
        let mut bounded = false;
        while !frontier.is_empty() {
            check()?;
            let remaining = request
                .limits
                .max_expanded_relationships
                .saturating_sub(*expanded);
            if expansions >= MAX_EXPANSIONS
                || remaining == 0
                || edges.len() >= request.limits.max_edges as usize
            {
                bounded = true;
                break;
            }
            let item = match request.traversal {
                DiscoveryTraversal::Bfs => frontier.pop_front(),
                DiscoveryTraversal::Dfs => frontier.pop_back(),
            };
            let Some((id, depth)) = item else { break };
            if depth >= request.limits.max_depth
                || depths.get(&id).is_some_and(|best| *best < depth)
            {
                continue;
            }
            expansions += 1;
            let owner = if let Some(budget) = metadata.as_deref_mut() {
                backend.node_for_link(&id, budget)?
            } else {
                backend.node_by_id(&id)?.map(std::borrow::Cow::Owned)
            };
            let Some(owner) = owner else {
                continue;
            };
            if !owner.kind.is_callable() || !relationships::exact_ast(&owner) {
                continue;
            }
            // Reserve one decoded record for lookahead, including when no output
            // record fits. Heuristic/rejected records consume the same allowance.
            let limit = remaining.saturating_sub(1).min(MAX_FANOUT as u64) as usize;
            let (calls, limited, examined) = if let Some(budget) = metadata.as_deref_mut() {
                backend.calls_for_link(&id, limit, budget)?
            } else {
                let (records, limited, examined) = backend.matching_bounded_counted(
                    &id,
                    false,
                    &[EdgeKind::Calls],
                    false,
                    limit,
                )?;
                (
                    records.into_iter().map(std::borrow::Cow::Owned).collect(),
                    limited,
                    examined,
                )
            };
            *expanded = expanded.saturating_add(examined as u64);
            bounded |= limited;
            for edge in calls {
                check()?;
                if edge.id.is_empty() || relationships::verified_site(&owner, &edge).is_none() {
                    continue;
                }
                if !edges.contains_key(&edge.id) && edges.len() >= request.limits.max_edges as usize
                {
                    bounded = true;
                    break;
                }
                let target = if let Some(budget) = metadata.as_deref_mut() {
                    backend.node_for_link(&edge.target, budget)?
                } else {
                    backend
                        .node_by_id(&edge.target)?
                        .map(std::borrow::Cow::Owned)
                };
                let Some(target) = target else {
                    continue;
                };
                if !target.kind.is_callable()
                    || !relationships::exact_ast(&target)
                    || !discovery_scope_matches(&target, scopes)
                {
                    continue;
                }
                if !nodes.contains_key(&target.id) {
                    if nodes.len() >= request.limits.max_nodes as usize {
                        bounded = true;
                        continue;
                    }
                    if let Some(budget) = metadata.as_deref_mut() {
                        budget.node(&target)?;
                    }
                    nodes.insert(target.id.clone(), query_node(&target));
                }
                if !edges.contains_key(&edge.id)
                    && let Some(budget) = metadata.as_deref_mut()
                {
                    budget.edge(&edge)?;
                }
                edges
                    .entry(edge.id.clone())
                    .or_insert_with(|| crate::discovery::discovery_edge(&edge));
                let next = depth + 1;
                if next < request.limits.max_depth
                    && depths
                        .get(&target.id)
                        .is_none_or(|previous| next < *previous)
                {
                    if frontier.len() >= MAX_FRONTIER {
                        bounded = true;
                        continue;
                    }
                    depths.insert(target.id.clone(), next);
                    frontier.push_back((target.id.clone(), next));
                }
            }
        }
        Ok(bounded)
    }
}

#[cfg(test)]
use crate::test_support;

#[cfg(test)]
mod tests {
    use super::*;
    use compass_model::code_graph::{GraphDocument, NodeKind};
    use std::cell::Cell;
    use std::error::Error;

    #[test]
    fn active_continuation_propagates_cancellation_and_deadline_errors()
    -> Result<(), Box<dyn Error>> {
        for kind in [QueryErrorKind::Cancelled, QueryErrorKind::Timeout] {
            let directory = tempfile::tempdir()?;
            let path = directory.path().join("graph.json");
            test_support::write_graph(&path)?;
            let mut graph = GraphDocument::load(&path)?;
            graph.nodes = [
                ("seed", "format_protocol_errors"),
                ("entry", "opaque"),
                ("leaf", "helper"),
            ]
            .into_iter()
            .map(|(id, name)| {
                test_support::node(id, NodeKind::Function, name, &format!("fixture.{name}"))
            })
            .collect();
            let template = graph
                .links
                .first()
                .cloned()
                .ok_or("missing edge template")?;
            graph.links = [("seed", "entry"), ("entry", "leaf")]
                .into_iter()
                .map(|(source, target)| {
                    let mut edge = template.clone();
                    edge.source = source.to_owned();
                    edge.target = target.to_owned();
                    edge.kind = EdgeKind::Calls;
                    edge.id = compass_model::identity::edge_id(
                        source,
                        edge.kind,
                        target,
                        edge.relationship_site.as_ref(),
                        edge.occurrence_rule.as_ref().map(|r| r.as_str()),
                    );
                    edge.key = edge.id.clone();
                    edge
                })
                .collect();
            std::fs::write(&path, serde_json::to_vec(&graph)?)?;
            let engine = crate::open(&path, None, &directory.path().join("cache"))?;
            let request = DiscoveryQueryRequest {
                question: "Where are protocol errors formatted?".to_owned(),
                direction: DiscoveryDirection::Auto,
                relation_contexts: Vec::new(),
                scope: Vec::new(),
                traversal: DiscoveryTraversal::Bfs,
                include_heuristic: false,
                limits: compass_model::query_contract::DiscoveryLimits {
                    max_depth: 3,
                    ..Default::default()
                },
            };
            let ranked = engine.discover_ranked(request.clone())?;
            assert_eq!(ranked.discovery.seeds.len(), 1);
            let backend = engine.backend.pin_discovery()?;
            let mut nodes = ranked
                .discovery
                .nodes
                .into_iter()
                .map(|n| (n.id.clone(), n))
                .collect();
            let mut edges = BTreeMap::new();
            let mut expanded = 0;
            let checks = Cell::new(0);
            let outcome = Context {
                scopes: &[],
                nodes: &mut nodes,
                edges: &mut edges,
                seeds: &ranked.discovery.seeds,
                expanded: &mut expanded,
                metadata: None,
            }
            .extend(&backend, &request, &|| {
                checks.set(checks.get() + 1);
                if checks.get() == 3 {
                    return Err(QueryError::new(
                        kind,
                        "TEST_GUARD",
                        "synthetic guard interrupted active continuation",
                    ));
                }
                Ok(())
            });
            assert!(matches!(outcome, Err(ref error) if error.kind() == kind));
            assert_eq!(checks.get(), 3);
            assert!(expanded > 0, "guard must interrupt after work starts");
            assert!(nodes.contains_key("entry"));
            assert!(!nodes.contains_key("leaf"));
            assert_eq!(edges.len(), 1);
        }
        Ok(())
    }
}
