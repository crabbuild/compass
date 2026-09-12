use std::collections::{BTreeMap, BTreeSet, VecDeque};

use thiserror::Error;

use crate::cluster::{PythonRandom, WeightedGraph};

const MODULARITY_THRESHOLD: f64 = 1e-4;

#[derive(Clone, Copy)]
struct MovePolicy<'a> {
    coarse: Option<&'a [usize]>,
    locked: Option<&'a BTreeSet<usize>>,
    preserve_source_connectivity: bool,
}

#[derive(Clone, Debug, Error, PartialEq)]
pub enum CommunityDetectorError {
    #[error(
        "community detector local_moves requires {required} moves, exceeds limit {limit} after {processed} moves"
    )]
    MoveLimitExceeded {
        required: usize,
        limit: usize,
        processed: usize,
    },
    #[error(
        "community detector levels requires {required} levels, exceeds limit {limit} after {processed} levels"
    )]
    LevelLimitExceeded {
        required: usize,
        limit: usize,
        processed: usize,
    },
    #[error("community detector produced a disconnected refined community")]
    DisconnectedPartition,
    #[error("community detector produced an incomplete partition")]
    IncompletePartition,
}

pub(crate) fn leiden(
    graph: &WeightedGraph,
    resolution: f64,
    max_levels: usize,
    max_moves: usize,
) -> Result<Vec<Vec<String>>, CommunityDetectorError> {
    if graph.edge_count() == 0 {
        return Ok(graph.ids.iter().cloned().map(|id| vec![id]).collect());
    }
    if max_levels == 0 {
        return Err(CommunityDetectorError::LevelLimitExceeded {
            required: 1,
            limit: 0,
            processed: 0,
        });
    }
    let mut current = graph.clone();
    let mut random = PythonRandom::seeded(42);
    let mut moves = 0usize;
    let mut best_modularity = f64::NEG_INFINITY;
    let mut best = current
        .members
        .iter()
        .map(|members| members.iter().cloned().collect::<Vec<_>>())
        .collect::<Vec<_>>();

    for level in 0..max_levels {
        let coarse = local_move(
            &current,
            resolution,
            MovePolicy {
                coarse: None,
                locked: None,
                preserve_source_connectivity: false,
            },
            &mut random,
            &mut moves,
            max_moves,
        )?;
        let coarse_assignment = assignment(&coarse, current.len());
        let refined = local_move(
            &current,
            resolution,
            MovePolicy {
                coarse: Some(&coarse_assignment),
                locked: None,
                preserve_source_connectivity: true,
            },
            &mut random,
            &mut moves,
            max_moves,
        )?;
        if !partition_connected(&current, &refined) {
            return Err(CommunityDetectorError::DisconnectedPartition);
        }
        let next_modularity = partition_modularity(&current, &refined, resolution);
        best = original_members(&current, &refined);
        if next_modularity - best_modularity <= MODULARITY_THRESHOLD
            || refined.len() == current.len()
        {
            break;
        }
        if level + 1 == max_levels {
            return Err(CommunityDetectorError::LevelLimitExceeded {
                required: max_levels.saturating_add(1),
                limit: max_levels,
                processed: max_levels,
            });
        }
        best_modularity = next_modularity;
        current = aggregate(&current, &refined);
        if current.len() <= 1 {
            break;
        }
    }
    for members in &mut best {
        members.sort();
    }
    best.sort_by(|left, right| right.len().cmp(&left.len()).then_with(|| left.cmp(right)));
    Ok(best)
}

pub(crate) fn leiden_anchored(
    graph: &WeightedGraph,
    resolution: f64,
    anchors: &BTreeSet<usize>,
    max_levels: usize,
    max_moves: usize,
) -> Result<Vec<Vec<String>>, CommunityDetectorError> {
    if graph.edge_count() == 0 {
        return Ok(graph.ids.iter().cloned().map(|id| vec![id]).collect());
    }
    if max_levels == 0 {
        return Err(CommunityDetectorError::LevelLimitExceeded {
            required: 1,
            limit: 0,
            processed: 0,
        });
    }
    let mut random = PythonRandom::seeded(42);
    let mut moves = 0usize;
    let coarse = local_move(
        graph,
        resolution,
        MovePolicy {
            coarse: None,
            locked: Some(anchors),
            preserve_source_connectivity: false,
        },
        &mut random,
        &mut moves,
        max_moves,
    )?;
    let coarse_assignment = assignment(&coarse, graph.len());
    let refined = local_move(
        graph,
        resolution,
        MovePolicy {
            coarse: Some(&coarse_assignment),
            locked: Some(anchors),
            preserve_source_connectivity: true,
        },
        &mut random,
        &mut moves,
        max_moves,
    )?;
    if !partition_connected(graph, &refined) {
        return Err(CommunityDetectorError::DisconnectedPartition);
    }
    let mut output = original_members(graph, &refined);
    for members in &mut output {
        members.sort();
    }
    output.sort_by(|left, right| right.len().cmp(&left.len()).then_with(|| left.cmp(right)));
    Ok(output)
}

fn local_move(
    graph: &WeightedGraph,
    resolution: f64,
    policy: MovePolicy<'_>,
    random: &mut PythonRandom,
    moves: &mut usize,
    max_moves: usize,
) -> Result<Vec<BTreeSet<usize>>, CommunityDetectorError> {
    let total_weight = graph.total_weight();
    let denominator = 2.0 * total_weight.powi(2);
    let degrees = (0..graph.len())
        .map(|node| graph.degree_weighted(node))
        .collect::<Vec<_>>();
    let mut node_to_community = (0..graph.len()).collect::<Vec<_>>();
    let mut members = (0..graph.len())
        .map(|node| BTreeSet::from([node]))
        .collect::<Vec<_>>();
    let mut totals = degrees.clone();
    let mut nodes = (0..graph.len()).collect::<Vec<_>>();
    random.shuffle(&mut nodes);
    let mut previous_modularity = partition_modularity(graph, &members, resolution);
    loop {
        let previous_members = members.clone();
        let mut pass_moves = 0usize;
        for node in &nodes {
            if policy.locked.is_some_and(|locked| locked.contains(node)) {
                continue;
            }
            let old = node_to_community[*node];
            // Refinement grows connected subcommunities by merging only a
            // singleton source into a neighboring group inside its coarse
            // community. The joining edge proves the destination remains
            // connected, while removing a singleton cannot disconnect its
            // source. This is both the Leiden refinement invariant and avoids
            // an O(VE) articulation search for every proposed move.
            if policy.preserve_source_connectivity && members[old].len() != 1 {
                continue;
            }
            let degree = degrees[*node];
            let mut neighbor_weights = BTreeMap::<usize, f64>::new();
            for (neighbor, weight) in graph.neighbors(*node) {
                if neighbor == node {
                    continue;
                }
                if policy
                    .coarse
                    .is_some_and(|partition| partition[*neighbor] != partition[*node])
                {
                    continue;
                }
                *neighbor_weights
                    .entry(node_to_community[*neighbor])
                    .or_default() += weight;
            }
            totals[old] -= degree;
            let old_weight = neighbor_weights.get(&old).copied().unwrap_or_default();
            let remove_cost =
                -old_weight / total_weight + resolution * totals[old] * degree / denominator;
            let mut best = old;
            let mut best_gain = 0.0;
            for (candidate, weight) in neighbor_weights {
                if candidate == old {
                    continue;
                }
                let gain = remove_cost + weight / total_weight
                    - resolution * totals[candidate] * degree / denominator;
                if gain > best_gain {
                    best = candidate;
                    best_gain = gain;
                }
            }
            totals[best] += degree;
            if best != old {
                if *moves == max_moves {
                    return Err(CommunityDetectorError::MoveLimitExceeded {
                        required: (*moves).saturating_add(1),
                        limit: max_moves,
                        processed: *moves,
                    });
                }
                members[old].remove(node);
                members[best].insert(*node);
                node_to_community[*node] = best;
                *moves += 1;
                pass_moves += 1;
            }
        }
        if pass_moves == 0 {
            break;
        }
        let modularity = partition_modularity(graph, &members, resolution);
        if modularity <= previous_modularity + f64::EPSILON {
            members = previous_members;
            break;
        }
        previous_modularity = modularity;
    }
    let mut retained = members
        .into_iter()
        .filter(|community| !community.is_empty())
        .collect::<Vec<_>>();
    retained.sort_by_key(|community| community.first().copied().unwrap_or_default());
    Ok(retained)
}

fn assignment(partition: &[BTreeSet<usize>], node_count: usize) -> Vec<usize> {
    let mut assignments = vec![usize::MAX; node_count];
    for (community, members) in partition.iter().enumerate() {
        for member in members {
            assignments[*member] = community;
        }
    }
    assignments
}

fn partition_connected(graph: &WeightedGraph, partition: &[BTreeSet<usize>]) -> bool {
    partition.iter().all(|community| {
        let Some(start) = community.first().copied() else {
            return true;
        };
        let mut visited = BTreeSet::from([start]);
        let mut queue = VecDeque::from([start]);
        while let Some(node) = queue.pop_front() {
            for (neighbor, _) in graph.neighbors(node) {
                if community.contains(neighbor) && visited.insert(*neighbor) {
                    queue.push_back(*neighbor);
                }
            }
        }
        visited.len() == community.len()
    })
}

fn partition_modularity(
    graph: &WeightedGraph,
    partition: &[BTreeSet<usize>],
    resolution: f64,
) -> f64 {
    let total_weight = graph.total_weight();
    if total_weight == 0.0 {
        return 0.0;
    }
    let assignments = assignment(partition, graph.len());
    let mut internal_weights = vec![0.0; partition.len()];
    for (left, right, weight) in graph.edges() {
        let community = assignments[left];
        if community != usize::MAX && community == assignments[right] {
            internal_weights[community] += weight;
        }
    }
    partition
        .iter()
        .enumerate()
        .map(|(community_index, community)| {
            let volume = community
                .iter()
                .map(|node| graph.degree_weighted(*node))
                .sum::<f64>();
            internal_weights[community_index] / total_weight
                - resolution * (volume / (2.0 * total_weight)).powi(2)
        })
        .sum()
}

fn original_members(graph: &WeightedGraph, partition: &[BTreeSet<usize>]) -> Vec<Vec<String>> {
    partition
        .iter()
        .map(|community| {
            community
                .iter()
                .flat_map(|node| graph.members[*node].iter().cloned())
                .collect()
        })
        .collect()
}

fn aggregate(graph: &WeightedGraph, partition: &[BTreeSet<usize>]) -> WeightedGraph {
    let assignments = assignment(partition, graph.len());
    let members = partition
        .iter()
        .map(|community| {
            community
                .iter()
                .flat_map(|node| graph.members[*node].iter().cloned())
                .collect()
        })
        .collect::<Vec<_>>();
    let ids = (0..partition.len()).map(|id| id.to_string()).collect();
    let mut output = WeightedGraph::new(ids, members);
    for (left, right, weight) in graph.edges() {
        output.add_edge(assignments[left], assignments[right], weight);
    }
    output
}

#[cfg(test)]
mod tests {
    use super::*;

    fn graph(edges: &[(usize, usize)]) -> WeightedGraph {
        let ids = (0..8).map(|node| node.to_string()).collect::<Vec<_>>();
        let members = ids.iter().cloned().map(|id| BTreeSet::from([id])).collect();
        let mut graph = WeightedGraph::new(ids, members);
        for (left, right) in edges {
            graph.add_edge(*left, *right, 1.0);
        }
        graph
    }

    fn reference_partition_modularity(
        graph: &WeightedGraph,
        partition: &[BTreeSet<usize>],
        resolution: f64,
    ) -> f64 {
        let total_weight = graph.total_weight();
        if total_weight == 0.0 {
            return 0.0;
        }
        partition
            .iter()
            .map(|community| {
                let internal = graph
                    .edges()
                    .filter(|(left, right, _)| {
                        community.contains(left) && community.contains(right)
                    })
                    .map(|(_, _, weight)| weight)
                    .sum::<f64>();
                let volume = community
                    .iter()
                    .map(|node| graph.degree_weighted(*node))
                    .sum::<f64>();
                internal / total_weight - resolution * (volume / (2.0 * total_weight)).powi(2)
            })
            .sum()
    }

    #[test]
    fn single_pass_modularity_matches_reference_edge_scans() {
        let graph = graph(&[
            (0, 0),
            (0, 1),
            (0, 2),
            (1, 2),
            (2, 3),
            (3, 4),
            (4, 5),
            (4, 6),
            (5, 6),
            (6, 7),
        ]);
        let partitions = [
            (0..8)
                .map(|node| BTreeSet::from([node]))
                .collect::<Vec<_>>(),
            vec![BTreeSet::from([0, 1, 2, 3]), BTreeSet::from([4, 5, 6, 7])],
            vec![
                BTreeSet::from([0, 1, 2]),
                BTreeSet::from([3, 4]),
                BTreeSet::from([5, 6, 7]),
            ],
        ];
        for resolution in [0.75, 1.0, 4.0 / 3.0] {
            for partition in &partitions {
                assert_eq!(
                    partition_modularity(&graph, partition, resolution).to_bits(),
                    reference_partition_modularity(&graph, partition, resolution).to_bits()
                );
            }
        }
    }

    #[test]
    fn separates_dense_groups_and_is_deterministic() -> Result<(), CommunityDetectorError> {
        let graph = graph(&[
            (0, 1),
            (0, 2),
            (0, 3),
            (1, 2),
            (1, 3),
            (2, 3),
            (4, 5),
            (4, 6),
            (4, 7),
            (5, 6),
            (5, 7),
            (6, 7),
            (3, 4),
        ]);
        let first = leiden(&graph, 1.0, 10, 10_000)?;
        let second = leiden(&graph, 1.0, 10, 10_000)?;
        assert_eq!(first, second);
        assert_eq!(first.len(), 2);
        assert_eq!(first[0], ["0", "1", "2", "3"]);
        assert_eq!(first[1], ["4", "5", "6", "7"]);
        Ok(())
    }

    #[test]
    fn respects_the_move_limit() {
        let graph = graph(&[(0, 1), (1, 2), (2, 3)]);
        assert!(matches!(
            leiden(&graph, 1.0, 10, 0),
            Err(CommunityDetectorError::MoveLimitExceeded { limit: 0, .. })
        ));
    }

    #[test]
    fn fails_closed_when_another_level_is_required() {
        let graph = graph(&[
            (0, 1),
            (0, 2),
            (0, 3),
            (1, 2),
            (1, 3),
            (2, 3),
            (4, 5),
            (4, 6),
            (4, 7),
            (5, 6),
            (5, 7),
            (6, 7),
            (3, 4),
        ]);
        assert!(matches!(
            leiden(&graph, 1.0, 1, 10_000),
            Err(CommunityDetectorError::LevelLimitExceeded {
                required: 2,
                limit: 1,
                processed: 1,
            })
        ));
    }

    #[test]
    fn articulation_fixture_never_returns_a_disconnected_community()
    -> Result<(), CommunityDetectorError> {
        let graph = graph(&[
            (0, 1),
            (0, 2),
            (1, 2),
            (2, 3),
            (3, 4),
            (4, 5),
            (4, 6),
            (5, 6),
        ]);
        let partition = leiden(&graph, 0.75, 10, 10_000)?;
        let positions = graph.position_map();
        let indexed = partition
            .iter()
            .map(|members| {
                members
                    .iter()
                    .map(|member| positions[member])
                    .collect::<BTreeSet<_>>()
            })
            .collect::<Vec<_>>();
        assert!(partition_connected(&graph, &indexed));
        Ok(())
    }
}
