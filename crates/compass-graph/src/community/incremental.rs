use std::collections::{BTreeMap, BTreeSet, HashMap};

use compass_model::code_graph::GraphDocument;

use super::leiden::{CommunityDetectorError, leiden_anchored};
use crate::cluster::{Communities, IncrementalClusterLimits, WeightedGraph};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum IncrementalPreparationFallback {
    InvalidLimits,
    RemovedNode,
    AffectedLimit,
    HubPolicy,
}

pub(crate) enum IncrementalPreparation {
    Ready(Box<AnchoredTopology>),
    Unchanged(Communities),
    Fallback {
        affected_nodes: usize,
        reason: IncrementalPreparationFallback,
    },
}

pub(crate) struct AnchoredTopology {
    graph: WeightedGraph,
    anchor_positions: BTreeSet<usize>,
    anchor_communities: BTreeMap<String, usize>,
    frozen: Communities,
    used_ids: BTreeSet<usize>,
    previous: HashMap<String, usize>,
    next_id: usize,
    pub affected_nodes: usize,
}

impl AnchoredTopology {
    pub fn cluster(
        &self,
        resolution: f64,
        max_levels: usize,
        max_moves: usize,
    ) -> Result<Option<Communities>, CommunityDetectorError> {
        let local = leiden_anchored(
            &self.graph,
            resolution,
            &self.anchor_positions,
            max_levels,
            max_moves,
        )?;
        let mut output = self.frozen.clone();
        let mut used = self.used_ids.clone();
        let mut next = self.next_id;
        for members in local {
            let anchors = members
                .iter()
                .filter_map(|member| self.anchor_communities.get(member).copied())
                .collect::<BTreeSet<_>>();
            if anchors.len() > 1 {
                return Ok(None);
            }
            let real_members = members
                .into_iter()
                .filter(|member| !self.anchor_communities.contains_key(member))
                .collect::<Vec<_>>();
            if real_members.is_empty() {
                continue;
            }
            let community = if let Some(anchor) = anchors.first().copied() {
                anchor
            } else {
                let mut overlaps = real_members
                    .iter()
                    .filter_map(|member| self.previous.get(member).copied())
                    .fold(BTreeMap::<usize, usize>::new(), |mut counts, community| {
                        *counts.entry(community).or_default() += 1;
                        counts
                    })
                    .into_iter()
                    .collect::<Vec<_>>();
                overlaps.sort_by_key(|(community, count)| (std::cmp::Reverse(*count), *community));
                if let Some(reused) = overlaps
                    .into_iter()
                    .map(|(community, _)| community)
                    .find(|community| used.insert(*community))
                {
                    reused
                } else {
                    while used.contains(&next) {
                        next = next.saturating_add(1);
                    }
                    let assigned = next;
                    used.insert(assigned);
                    next = next.saturating_add(1);
                    assigned
                }
            };
            output.entry(community).or_default().extend(real_members);
        }
        output.retain(|_, members| !members.is_empty());
        for members in output.values_mut() {
            members.sort();
            members.dedup();
        }
        Ok(Some(output))
    }
}

pub(crate) fn prepare_anchored_topology(
    document: &GraphDocument,
    graph: &WeightedGraph,
    previous: &HashMap<String, usize>,
    changed_sources: &BTreeSet<String>,
    limits: IncrementalClusterLimits,
    exclude_hubs_percentile: Option<f64>,
) -> IncrementalPreparation {
    if exclude_hubs_percentile.is_some() {
        return IncrementalPreparation::Fallback {
            affected_nodes: graph.len(),
            reason: IncrementalPreparationFallback::HubPolicy,
        };
    }
    if limits.max_affected_nodes == 0
        || !limits.max_affected_fraction.is_finite()
        || limits.max_affected_fraction <= 0.0
    {
        return IncrementalPreparation::Fallback {
            affected_nodes: graph.len(),
            reason: IncrementalPreparationFallback::InvalidLimits,
        };
    }
    let graph_positions = graph.position_map();
    if previous
        .keys()
        .any(|node| !graph_positions.contains_key(node))
    {
        return IncrementalPreparation::Fallback {
            affected_nodes: graph.len(),
            reason: IncrementalPreparationFallback::RemovedNode,
        };
    }
    let sources = document
        .nodes
        .iter()
        .map(|node| {
            (
                node.id.as_str(),
                node.source
                    .as_ref()
                    .map(|source| source.file.replace('\\', "/")),
            )
        })
        .collect::<BTreeMap<_, _>>();
    let mut affected = graph
        .ids
        .iter()
        .enumerate()
        .filter_map(|(position, id)| {
            let changed = sources
                .get(id.as_str())
                .and_then(|source| source.as_ref())
                .is_some_and(|source| changed_sources.contains(source));
            (!previous.contains_key(id) || changed).then_some(position)
        })
        .collect::<BTreeSet<_>>();
    if affected.is_empty() {
        return IncrementalPreparation::Unchanged(communities_from_previous(graph, previous));
    }
    let touched = affected
        .iter()
        .filter_map(|position| previous.get(&graph.ids[*position]).copied())
        .collect::<BTreeSet<_>>();
    for (position, id) in graph.ids.iter().enumerate() {
        if previous
            .get(id)
            .is_some_and(|community| touched.contains(community))
        {
            affected.insert(position);
        }
    }
    let fraction_limit =
        ((graph.len() as f64 * limits.max_affected_fraction).ceil() as usize).max(1);
    let affected_limit = limits.max_affected_nodes.min(fraction_limit);
    if affected.len() > affected_limit {
        return IncrementalPreparation::Fallback {
            affected_nodes: affected.len(),
            reason: IncrementalPreparationFallback::AffectedLimit,
        };
    }

    let mut adjacent_frozen = BTreeSet::<usize>::new();
    for (left, right, _) in graph.edges() {
        if affected.contains(&left) && !affected.contains(&right) {
            if let Some(community) = previous.get(&graph.ids[right]) {
                adjacent_frozen.insert(*community);
            }
        } else if affected.contains(&right)
            && !affected.contains(&left)
            && let Some(community) = previous.get(&graph.ids[left])
        {
            adjacent_frozen.insert(*community);
        }
    }
    let mut ids = affected
        .iter()
        .map(|position| graph.ids[*position].clone())
        .collect::<Vec<_>>();
    let anchor_ids = adjacent_frozen
        .iter()
        .map(|community| {
            (
                *community,
                format!("\0compass-community-anchor:{community}"),
            )
        })
        .collect::<BTreeMap<_, _>>();
    ids.extend(anchor_ids.values().cloned());
    let members = ids.iter().cloned().map(|id| BTreeSet::from([id])).collect();
    let mut local = WeightedGraph::new(ids.clone(), members);
    let local_positions = ids
        .iter()
        .enumerate()
        .map(|(position, id)| (id.as_str(), position))
        .collect::<BTreeMap<_, _>>();
    let full_to_local = affected
        .iter()
        .map(|position| (*position, local_positions[graph.ids[*position].as_str()]))
        .collect::<BTreeMap<_, _>>();
    for (left, right, weight) in graph.edges() {
        match (full_to_local.get(&left), full_to_local.get(&right)) {
            (Some(local_left), Some(local_right)) => {
                local.add_edge(*local_left, *local_right, weight);
            }
            (Some(local_node), None) => {
                if let Some(community) = previous.get(&graph.ids[right])
                    && let Some(anchor) = anchor_ids.get(community)
                {
                    local.add_edge(*local_node, local_positions[anchor.as_str()], weight);
                }
            }
            (None, Some(local_node)) => {
                if let Some(community) = previous.get(&graph.ids[left])
                    && let Some(anchor) = anchor_ids.get(community)
                {
                    local.add_edge(local_positions[anchor.as_str()], *local_node, weight);
                }
            }
            (None, None) => {}
        }
    }
    let anchor_communities = anchor_ids
        .into_iter()
        .map(|(community, id)| (id, community))
        .collect::<BTreeMap<_, _>>();
    let anchor_positions = anchor_communities
        .keys()
        .map(|id| local_positions[id.as_str()])
        .collect();
    let frozen = previous
        .iter()
        .filter_map(|(id, community)| {
            let position = graph_positions.get(id)?;
            (!affected.contains(position)).then_some((*community, id.clone()))
        })
        .fold(Communities::new(), |mut output, (community, id)| {
            output.entry(community).or_default().push(id);
            output
        });
    let used_ids = frozen.keys().copied().collect::<BTreeSet<_>>();
    let next_id = previous
        .values()
        .copied()
        .max()
        .map_or(0, |maximum| maximum.saturating_add(1));
    IncrementalPreparation::Ready(Box::new(AnchoredTopology {
        graph: local,
        anchor_positions,
        anchor_communities,
        frozen,
        used_ids,
        previous: previous.clone(),
        next_id,
        affected_nodes: affected.len(),
    }))
}

pub(crate) fn baseline_partition(
    graph: &WeightedGraph,
    previous: &HashMap<String, usize>,
) -> Communities {
    let mut output = communities_from_previous(graph, previous);
    let mut used = output.keys().copied().collect::<BTreeSet<_>>();
    let mut next = previous
        .values()
        .copied()
        .max()
        .map_or(0, |maximum| maximum.saturating_add(1));
    for id in &graph.ids {
        if previous.contains_key(id) {
            continue;
        }
        while used.contains(&next) {
            next = next.saturating_add(1);
        }
        output.insert(next, vec![id.clone()]);
        used.insert(next);
        next = next.saturating_add(1);
    }
    output
}

fn communities_from_previous(
    graph: &WeightedGraph,
    previous: &HashMap<String, usize>,
) -> Communities {
    let mut output = Communities::new();
    for id in &graph.ids {
        if let Some(community) = previous.get(id) {
            output.entry(*community).or_default().push(id.clone());
        }
    }
    output
}
