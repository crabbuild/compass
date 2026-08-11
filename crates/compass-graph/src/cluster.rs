use std::collections::{BTreeMap, BTreeSet};
use std::time::Instant;

use ahash::{AHashMap as HashMap, AHashSet as HashSet};
use compass_model::{EdgeRecord, GraphDocument, NodeRecord};
use rayon::prelude::*;
use serde_json::{Map, Value};
use sha2::{Digest, Sha256};

const MAX_COMMUNITY_FRACTION: f64 = 0.25;
const MIN_SPLIT_SIZE: usize = 10;
const COHESION_SPLIT_THRESHOLD: f64 = 0.05;
const COHESION_SPLIT_MIN_SIZE: usize = 50;
const LOUVAIN_THRESHOLD: f64 = 1e-4;
const LOUVAIN_MAX_LEVEL: usize = 10;

pub type Communities = BTreeMap<usize, Vec<String>>;

/// Bounds for a topology-changing incremental community update.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct IncrementalClusterLimits {
    /// Absolute ceiling for nodes admitted to the local reclustering region.
    pub max_affected_nodes: usize,
    /// Fractional ceiling relative to the current graph.
    pub max_affected_fraction: f64,
}

impl Default for IncrementalClusterLimits {
    fn default() -> Self {
        Self {
            max_affected_nodes: 4_096,
            max_affected_fraction: 0.25,
        }
    }
}

/// Community result plus evidence that work stayed within the local bound.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct IncrementalClusterResult {
    pub communities: Communities,
    pub affected_nodes: usize,
    pub used_incremental: bool,
}

#[derive(Clone, Debug)]
struct CommunityLabelCandidate {
    community: usize,
    base: String,
    context: Option<String>,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ClusterOptions {
    pub resolution: f64,
    pub exclude_hubs_percentile: Option<f64>,
}

impl Default for ClusterOptions {
    fn default() -> Self {
        Self {
            resolution: 1.0,
            exclude_hubs_percentile: None,
        }
    }
}

/// Detect stable communities with a native port of NetworkX's seeded Louvain pass.
#[must_use]
pub fn cluster(document: &GraphDocument, options: ClusterOptions) -> Communities {
    let mut profile_started = Instant::now();
    let graph = WeightedGraph::from_document(document);
    profile_cluster("weighted graph construction", &mut profile_started);
    if graph.is_empty() {
        return Communities::new();
    }
    if graph.edge_count() == 0 {
        return graph
            .ids
            .iter()
            .enumerate()
            .map(|(index, id)| (index, vec![id.clone()]))
            .collect();
    }

    let hubs = excluded_hubs(&graph, options.exclude_hubs_percentile);
    let isolates = (0..graph.len())
        .filter(|node| graph.degree_unweighted(*node) == 0 && !hubs.contains(node))
        .collect::<Vec<_>>();
    let connected_nodes = (0..graph.len())
        .filter(|node| graph.degree_unweighted(*node) > 0 && !hubs.contains(node))
        .collect::<Vec<_>>();
    let connected = graph.subgraph(&connected_nodes);
    profile_cluster("hub filtering and connected subgraph", &mut profile_started);

    let mut raw = Vec::<Vec<String>>::new();
    if !connected.is_empty() {
        raw.extend(louvain(&connected, options.resolution));
    }
    profile_cluster("Louvain levels", &mut profile_started);
    raw.extend(
        isolates
            .into_iter()
            .map(|node| vec![graph.ids[node].clone()]),
    );

    if !hubs.is_empty() {
        reattach_hubs(&graph, &hubs, &mut raw);
    }
    profile_cluster("isolate and hub attachment", &mut profile_started);

    let positions = graph.position_map();
    let maximum_size = MIN_SPLIT_SIZE.max((graph.len() as f64 * MAX_COMMUNITY_FRACTION) as usize);
    let first_pass = raw
        .into_par_iter()
        .map(|members| {
            if members.len() > maximum_size {
                split_community(&graph, &positions, &members)
            } else {
                vec![members]
            }
        })
        .collect::<Vec<_>>()
        .into_iter()
        .flatten()
        .collect::<Vec<_>>();
    let mut final_communities = first_pass
        .into_par_iter()
        .map(|members| {
            if members.len() >= COHESION_SPLIT_MIN_SIZE
                && cohesion_score_graph(&graph, &positions, &members) < COHESION_SPLIT_THRESHOLD
            {
                let splits = split_community(&graph, &positions, &members);
                if splits.len() > 1 {
                    splits
                } else {
                    vec![members]
                }
            } else {
                vec![members]
            }
        })
        .collect::<Vec<_>>()
        .into_iter()
        .flatten()
        .collect::<Vec<_>>();
    for members in &mut final_communities {
        members.sort();
    }
    final_communities
        .sort_by(|left, right| right.len().cmp(&left.len()).then_with(|| left.cmp(right)));
    let communities = final_communities.into_iter().enumerate().collect();
    profile_cluster("community splitting and ordering", &mut profile_started);
    communities
}

/// Recluster only communities touched by changed source files and their
/// immediate topology boundary.
///
/// Unaffected assignments are frozen. If the affected region exceeds either
/// configured ceiling, this function falls back to the complete algorithm so
/// a large refactor cannot be mislabeled as a local update.
#[must_use]
pub fn cluster_incremental(
    document: &GraphDocument,
    previous: &std::collections::HashMap<String, usize>,
    changed_sources: &BTreeSet<String>,
    options: ClusterOptions,
    limits: IncrementalClusterLimits,
) -> IncrementalClusterResult {
    if previous.is_empty()
        || changed_sources.is_empty()
        || limits.max_affected_nodes == 0
        || !limits.max_affected_fraction.is_finite()
        || limits.max_affected_fraction <= 0.0
    {
        return full_cluster_result(document, previous, options);
    }

    let positions = document
        .nodes
        .iter()
        .enumerate()
        .map(|(index, node)| (node.id.as_str(), index))
        .collect::<HashMap<_, _>>();
    if previous
        .keys()
        .any(|node_id| !positions.contains_key(node_id.as_str()))
    {
        return full_cluster_result(document, previous, options);
    }
    let current_sources = document
        .nodes
        .iter()
        .map(|node| node.string("source_file").replace('\\', "/"))
        .filter(|source| !source.is_empty())
        .collect::<HashSet<_>>();
    if changed_sources
        .iter()
        .any(|source| !current_sources.contains(source))
    {
        return full_cluster_result(document, previous, options);
    }
    let mut affected = document
        .nodes
        .iter()
        .enumerate()
        .filter_map(|(index, node)| {
            let source = node.string("source_file").replace('\\', "/");
            (!previous.contains_key(&node.id) || changed_sources.contains(&source)).then_some(index)
        })
        .collect::<HashSet<_>>();
    if affected.is_empty() {
        return IncrementalClusterResult {
            communities: communities_from_assignments(document, previous),
            affected_nodes: 0,
            used_incremental: true,
        };
    }

    let touched_communities = affected
        .iter()
        .filter_map(|index| previous.get(&document.nodes[*index].id).copied())
        .collect::<HashSet<_>>();
    for (index, node) in document.nodes.iter().enumerate() {
        if previous
            .get(&node.id)
            .is_some_and(|community| touched_communities.contains(community))
        {
            affected.insert(index);
        }
    }
    let boundary = document
        .links
        .iter()
        .filter_map(|edge| {
            let source = positions.get(edge.source.as_str()).copied()?;
            let target = positions.get(edge.target.as_str()).copied()?;
            (affected.contains(&source) || affected.contains(&target)).then_some([source, target])
        })
        .flatten()
        .collect::<Vec<_>>();
    affected.extend(boundary);
    let boundary_communities = affected
        .iter()
        .filter_map(|index| previous.get(&document.nodes[*index].id).copied())
        .collect::<HashSet<_>>();
    for (index, node) in document.nodes.iter().enumerate() {
        if previous
            .get(&node.id)
            .is_some_and(|community| boundary_communities.contains(community))
        {
            affected.insert(index);
        }
    }

    let fraction_limit =
        ((document.nodes.len() as f64 * limits.max_affected_fraction).ceil() as usize).max(1);
    let affected_limit = limits.max_affected_nodes.min(fraction_limit);
    if affected.len() > affected_limit {
        return full_cluster_result(document, previous, options);
    }

    let affected_ids = affected
        .iter()
        .map(|index| document.nodes[*index].id.as_str())
        .collect::<HashSet<_>>();
    let local_document = GraphDocument {
        directed: document.directed,
        multigraph: document.multigraph,
        graph: Map::new(),
        nodes: affected
            .iter()
            .map(|index| document.nodes[*index].clone())
            .collect(),
        links: document
            .links
            .iter()
            .filter(|edge| {
                affected_ids.contains(edge.source.as_str())
                    && affected_ids.contains(edge.target.as_str())
            })
            .cloned()
            .collect(),
        extras: BTreeMap::new(),
    };
    let local = cluster(&local_document, options);
    let local_assignments = stable_local_assignments(&local, previous, &affected_ids);
    let mut assignments = previous
        .iter()
        .filter(|(node, _)| {
            positions.contains_key(node.as_str()) && !affected_ids.contains(node.as_str())
        })
        .map(|(node, community)| (node.clone(), *community))
        .collect::<std::collections::HashMap<_, _>>();
    assignments.extend(local_assignments);

    IncrementalClusterResult {
        communities: communities_from_assignments(document, &assignments),
        affected_nodes: affected.len(),
        used_incremental: true,
    }
}

fn full_cluster_result(
    document: &GraphDocument,
    previous: &std::collections::HashMap<String, usize>,
    options: ClusterOptions,
) -> IncrementalClusterResult {
    let current = cluster(document, options);
    IncrementalClusterResult {
        communities: if previous.is_empty() {
            current
        } else {
            remap_communities_to_previous(&current, previous)
        },
        affected_nodes: document.nodes.len(),
        used_incremental: false,
    }
}

fn stable_local_assignments(
    local: &Communities,
    previous: &std::collections::HashMap<String, usize>,
    affected_ids: &HashSet<&str>,
) -> std::collections::HashMap<String, usize> {
    let unaffected_ids = previous
        .iter()
        .filter(|(node, _)| !affected_ids.contains(node.as_str()))
        .map(|(_, community)| *community)
        .collect::<HashSet<_>>();
    let mut used = unaffected_ids;
    let mut next = previous
        .values()
        .copied()
        .max()
        .map_or(0, |maximum| maximum.saturating_add(1));
    let mut assignments = std::collections::HashMap::new();
    for members in local.values() {
        let mut overlaps = members
            .iter()
            .filter_map(|member| previous.get(member).copied())
            .fold(BTreeMap::<usize, usize>::new(), |mut counts, community| {
                *counts.entry(community).or_default() += 1;
                counts
            })
            .into_iter()
            .collect::<Vec<_>>();
        overlaps.sort_by_key(|(community, count)| (std::cmp::Reverse(*count), *community));
        let community = overlaps
            .into_iter()
            .map(|(community, _)| community)
            .find(|community| used.insert(*community))
            .unwrap_or_else(|| {
                while used.contains(&next) {
                    next = next.saturating_add(1);
                }
                let assigned = next;
                used.insert(assigned);
                next = next.saturating_add(1);
                assigned
            });
        for member in members {
            assignments.insert(member.clone(), community);
        }
    }
    assignments
}

fn communities_from_assignments(
    document: &GraphDocument,
    assignments: &std::collections::HashMap<String, usize>,
) -> Communities {
    let mut communities = Communities::new();
    let mut next = assignments
        .values()
        .copied()
        .max()
        .map_or(0, |maximum| maximum.saturating_add(1));
    for node in &document.nodes {
        let community = assignments.get(&node.id).copied().unwrap_or_else(|| {
            let assigned = next;
            next = next.saturating_add(1);
            assigned
        });
        communities
            .entry(community)
            .or_default()
            .push(node.id.clone());
    }
    for members in communities.values_mut() {
        members.sort();
    }
    communities
}

fn profile_cluster(label: &str, started: &mut Instant) {
    if std::env::var_os("COMPASS_PROFILE_INTERNAL").is_some() {
        eprintln!(
            "[compass internal] cluster {label}: {:.3}s",
            started.elapsed().as_secs_f64()
        );
    }
    *started = Instant::now();
}

#[must_use]
pub fn label_communities_by_hub(
    document: &GraphDocument,
    communities: &Communities,
) -> BTreeMap<usize, String> {
    let positions = document
        .nodes
        .iter()
        .enumerate()
        .map(|(index, node)| (node.id.as_str(), index))
        .collect::<HashMap<_, _>>();
    let mut degrees = vec![0_usize; document.nodes.len()];
    let mut neighbors = HashSet::<(usize, usize)>::new();
    for edge in &document.links {
        let (Some(&left), Some(&right)) = (
            positions.get(edge.source.as_str()),
            positions.get(edge.target.as_str()),
        ) else {
            continue;
        };
        let pair = if left <= right {
            (left, right)
        } else {
            (right, left)
        };
        if !neighbors.insert(pair) {
            continue;
        }
        if left == right {
            degrees[left] += 2;
        } else {
            degrees[left] += 1;
            degrees[right] += 1;
        }
    }
    let mut candidates = Vec::with_capacity(communities.len());
    for (community, members) in communities {
        let hub = members
            .iter()
            .filter_map(|member| positions.get(member.as_str()).map(|index| (member, *index)))
            .min_by(|(left_id, left), (right_id, right)| {
                degrees[*right]
                    .cmp(&degrees[*left])
                    .then_with(|| left_id.cmp(right_id))
            });
        let fallback = format!("Community {community}");
        let (base, context) = hub
            .and_then(|(_, index)| document.nodes.get(index))
            .map(|node| {
                (
                    concise_community_label(node).unwrap_or_else(|| fallback.clone()),
                    community_label_context(node),
                )
            })
            .unwrap_or((fallback, None));
        candidates.push(CommunityLabelCandidate {
            community: *community,
            base,
            context,
        });
    }

    let mut base_counts = HashMap::<String, usize>::new();
    for candidate in &candidates {
        *base_counts.entry(candidate.base.clone()).or_insert(0) += 1;
    }
    let mut labels = candidates
        .into_iter()
        .map(|candidate| {
            let duplicate = base_counts.get(&candidate.base).copied().unwrap_or(0) > 1;
            let label = if duplicate {
                candidate.context.map_or_else(
                    || format!("{} (community {})", candidate.base, candidate.community),
                    |context| format!("{} ({context})", candidate.base),
                )
            } else {
                candidate.base
            };
            (candidate.community, label)
        })
        .collect::<Vec<_>>();

    let mut contextual_counts = HashMap::<String, usize>::new();
    for (_, label) in &labels {
        *contextual_counts.entry(label.clone()).or_insert(0) += 1;
    }
    for (community, label) in &mut labels {
        if contextual_counts.get(label).copied().unwrap_or(0) > 1 {
            label.push_str(&format!(" [community {community}]"));
        }
    }

    // An untrusted source label can resemble a generated contextual label. In
    // that exceptional case, suffix every label once. A trailing unique
    // community ID guarantees uniqueness without an unbounded retry loop.
    let mut unique = HashSet::with_capacity(labels.len());
    if !labels
        .iter()
        .all(|(_, label)| unique.insert(label.as_str()))
    {
        for (community, label) in &mut labels {
            label.push_str(&format!(" [community {community}]"));
        }
    }

    labels.into_iter().collect()
}

fn concise_community_label(node: &NodeRecord) -> Option<String> {
    let label = node.label().trim();
    let label = label.strip_suffix("()").unwrap_or(label).trim();
    (!label.is_empty()).then(|| label.to_owned())
}

fn community_label_context(node: &NodeRecord) -> Option<String> {
    if let Some(file) = node
        .source_file()
        .map(str::trim)
        .filter(|file| !file.is_empty())
    {
        let location = node
            .unsigned("line_start")
            .map(|line| format!("L{line}"))
            .or_else(|| concise_location(&node.string("source_location")));
        return Some(community_anchor_label(file, location.as_deref()));
    }

    let wiring_file = node.string("wiring_file");
    if !wiring_file.trim().is_empty() {
        let wiring_location = concise_location(&node.string("wiring_location"));
        return Some(community_anchor_label(
            wiring_file.trim(),
            wiring_location.as_deref(),
        ));
    }

    for key in ["qualifiedName", "qualified_name", "signature"] {
        let value = node.string(key);
        let value = value.trim();
        if !value.is_empty() && value != node.label().trim() {
            return Some(value.to_owned());
        }
    }
    None
}

fn concise_location(location: &str) -> Option<String> {
    let location = location.trim();
    if let Some(rest) = location.strip_prefix('L') {
        let digits = rest
            .chars()
            .take_while(char::is_ascii_digit)
            .collect::<String>();
        if !digits.is_empty() {
            return Some(format!("L{digits}"));
        }
    }
    (!location.is_empty()).then(|| location.to_owned())
}

fn community_anchor_label(file: &str, location: Option<&str>) -> String {
    let mut components = file
        .rsplit(['/', '\\'])
        .filter(|component| !component.is_empty());
    let name = components.next().unwrap_or(file);
    let path = components
        .next()
        .map_or_else(|| name.to_owned(), |parent| format!("{parent}/{name}"));
    location.map_or(path.clone(), |location| format!("{path}:{location}"))
}

#[must_use]
pub fn community_member_signatures(communities: &Communities) -> BTreeMap<usize, String> {
    communities
        .iter()
        .map(|(community, members)| {
            let mut sorted = members.clone();
            sorted.sort();
            let mut hasher = Sha256::new();
            for member in sorted {
                hasher.update(member.as_bytes());
                hasher.update([0]);
            }
            let digest = format!("{:x}", hasher.finalize());
            (*community, digest[..16].to_owned())
        })
        .collect()
}

#[must_use]
pub fn cohesion_score(document: &GraphDocument, members: &[String]) -> f64 {
    let graph = WeightedGraph::from_document(document);
    let positions = graph.position_map();
    cohesion_score_graph(&graph, &positions, members)
}

#[must_use]
pub fn score_communities(
    document: &GraphDocument,
    communities: &Communities,
) -> BTreeMap<usize, f64> {
    let positions = document
        .nodes
        .iter()
        .enumerate()
        .map(|(index, node)| (node.id.as_str(), index))
        .collect::<HashMap<_, _>>();
    let mut node_community = vec![None; document.nodes.len()];
    let mut internal_edges = HashMap::<usize, usize>::new();
    for (community, members) in communities {
        for member in members {
            if let Some(position) = positions.get(member.as_str()) {
                node_community[*position] = Some(*community);
            }
        }
    }
    let mut seen = HashSet::<(usize, usize)>::new();
    for edge in &document.links {
        let (Some(&left), Some(&right)) = (
            positions.get(edge.source.as_str()),
            positions.get(edge.target.as_str()),
        ) else {
            continue;
        };
        let pair = if left <= right {
            (left, right)
        } else {
            (right, left)
        };
        if seen.insert(pair)
            && let Some(community) = node_community[left]
            && node_community[right] == Some(community)
        {
            *internal_edges.entry(community).or_default() += 1;
        }
    }
    communities
        .iter()
        .map(|(community, members)| {
            let count = members.len();
            let possible = count.saturating_mul(count.saturating_sub(1)) / 2;
            let score = if possible == 0 {
                1.0
            } else {
                internal_edges.get(community).copied().unwrap_or_default() as f64 / possible as f64
            };
            (*community, score)
        })
        .collect()
}

#[must_use]
pub fn remap_communities_to_previous(
    communities: &Communities,
    previous: &std::collections::HashMap<String, usize>,
) -> Communities {
    if communities.is_empty() {
        return Communities::new();
    }
    let mut overlap_counts = HashMap::<(usize, usize), usize>::new();
    for (new_community, nodes) in communities {
        for node in nodes {
            if let Some(old_community) = previous.get(node) {
                *overlap_counts
                    .entry((*old_community, *new_community))
                    .or_default() += 1;
            }
        }
    }
    let mut overlaps = overlap_counts
        .into_iter()
        .map(|((old, new), overlap)| (overlap, old, new))
        .collect::<Vec<_>>();
    overlaps.sort_by_key(|(overlap, old, new)| (std::cmp::Reverse(*overlap), *old, *new));
    let mut mapping = HashMap::new();
    let mut used_old = HashSet::new();
    let mut matched_new = HashSet::new();
    for (_, old, new) in overlaps {
        if used_old.insert(old) && matched_new.insert(new) {
            mapping.insert(new, old);
        }
    }
    let mut unmatched = communities
        .iter()
        .filter(|(community, _)| !matched_new.contains(community))
        .collect::<Vec<_>>();
    unmatched.sort_by(|(left_id, left), (right_id, right)| {
        right
            .len()
            .cmp(&left.len())
            .then_with(|| {
                let mut left = (*left).clone();
                let mut right = (*right).clone();
                left.sort();
                right.sort();
                left.cmp(&right)
            })
            .then_with(|| left_id.cmp(right_id))
    });
    let mut next = 0;
    for (community, _) in unmatched {
        while used_old.contains(&next) {
            next += 1;
        }
        mapping.insert(*community, next);
        used_old.insert(next);
        next += 1;
    }
    communities
        .iter()
        .filter_map(|(community, members)| {
            let final_id = mapping.get(community)?;
            let mut members = members.clone();
            members.sort();
            Some((*final_id, members))
        })
        .collect()
}

fn excluded_hubs(graph: &WeightedGraph, percentile: Option<f64>) -> HashSet<usize> {
    let Some(percentile) = percentile else {
        return HashSet::new();
    };
    let mut degrees = (0..graph.len())
        .map(|node| graph.degree_unweighted(node))
        .collect::<Vec<_>>();
    degrees.sort_unstable();
    if degrees.is_empty() {
        return HashSet::new();
    }
    let index = (((degrees.len() as f64 * percentile / 100.0) as isize) - 1)
        .max(0)
        .cast_unsigned()
        .min(degrees.len() - 1);
    let threshold = degrees[index];
    (0..graph.len())
        .filter(|node| graph.degree_unweighted(*node) > threshold)
        .collect()
}

fn reattach_hubs(graph: &WeightedGraph, hubs: &HashSet<usize>, raw: &mut Vec<Vec<String>>) {
    let mut node_community = raw
        .iter()
        .enumerate()
        .flat_map(|(community, members)| {
            members
                .iter()
                .map(move |member| (member.clone(), community))
        })
        .collect::<HashMap<_, _>>();
    let mut hubs = hubs.iter().copied().collect::<Vec<_>>();
    hubs.sort_by_key(|node| &graph.ids[*node]);
    for hub in hubs {
        let mut votes = HashMap::<usize, usize>::new();
        for (neighbor, _) in &graph.adjacency[hub] {
            if let Some(community) = node_community.get(&graph.ids[*neighbor]) {
                *votes.entry(*community).or_default() += 1;
            }
        }
        let best = votes
            .into_iter()
            .min_by_key(|(community, votes)| (std::cmp::Reverse(*votes), *community))
            .map(|(community, _)| community);
        if let Some(community) = best {
            raw[community].push(graph.ids[hub].clone());
            node_community.insert(graph.ids[hub].clone(), community);
        } else {
            let community = raw.len();
            raw.push(vec![graph.ids[hub].clone()]);
            node_community.insert(graph.ids[hub].clone(), community);
        }
    }
}

fn split_community(
    graph: &WeightedGraph,
    positions: &HashMap<&String, usize>,
    members: &[String],
) -> Vec<Vec<String>> {
    let selected = members
        .iter()
        .filter_map(|member| positions.get(member).copied())
        .collect::<Vec<_>>();
    let subgraph = graph.subgraph(&selected);
    if subgraph.edge_count() == 0 {
        let mut output = members
            .iter()
            .cloned()
            .map(|member| vec![member])
            .collect::<Vec<_>>();
        output.sort();
        return output;
    }
    let communities = louvain(&subgraph, 1.0);
    if communities.len() <= 1 {
        let mut members = members.to_vec();
        members.sort();
        vec![members]
    } else {
        communities
    }
}

fn cohesion_score_graph(
    graph: &WeightedGraph,
    positions: &HashMap<&String, usize>,
    members: &[String],
) -> f64 {
    let count = members.len();
    if count <= 1 {
        return 1.0;
    }
    let member_set = members
        .iter()
        .filter_map(|member| positions.get(member).copied())
        .collect::<HashSet<_>>();
    let actual = member_set
        .iter()
        .map(|left| {
            graph.adjacency[*left]
                .iter()
                .filter(|(right, _)| left <= right && member_set.contains(right))
                .count()
        })
        .sum::<usize>();
    let possible = count * (count - 1) / 2;
    actual as f64 / possible as f64
}

fn louvain(graph: &WeightedGraph, resolution: f64) -> Vec<Vec<String>> {
    if graph.edge_count() == 0 {
        return graph.ids.iter().cloned().map(|id| vec![id]).collect();
    }
    let mut random = PythonRandom::seeded(42);
    let mut current = graph.clone();
    let partition = graph
        .ids
        .iter()
        .cloned()
        .map(|id| BTreeSet::from([id]))
        .collect::<Vec<_>>();
    let mut previous_modularity = modularity(
        &current,
        &(0..current.len())
            .map(|node| BTreeSet::from([node]))
            .collect::<Vec<_>>(),
        resolution,
    );
    let total_weight = current.total_weight();
    let (mut next_partition, mut inner, _) =
        one_level(&current, total_weight, partition, resolution, &mut random);
    let mut final_partition = next_partition.clone();
    for _ in 0..LOUVAIN_MAX_LEVEL {
        final_partition.clone_from(&next_partition);
        let next_modularity = modularity(&current, &inner, resolution);
        if next_modularity - previous_modularity <= LOUVAIN_THRESHOLD {
            break;
        }
        previous_modularity = next_modularity;
        current = aggregate_graph(&current, &inner);
        let (partition_after, inner_after, improved) = one_level(
            &current,
            total_weight,
            next_partition,
            resolution,
            &mut random,
        );
        if !improved {
            break;
        }
        next_partition = partition_after;
        inner = inner_after;
    }
    final_partition
        .into_iter()
        .map(|members| members.into_iter().collect())
        .collect()
}

fn one_level(
    graph: &WeightedGraph,
    total_weight: f64,
    mut partition: Vec<BTreeSet<String>>,
    resolution: f64,
    random: &mut PythonRandom,
) -> (Vec<BTreeSet<String>>, Vec<BTreeSet<usize>>, bool) {
    let mut node_to_community = (0..graph.len()).collect::<Vec<_>>();
    let mut inner = (0..graph.len())
        .map(|node| BTreeSet::from([node]))
        .collect::<Vec<_>>();
    let degrees = (0..graph.len())
        .map(|node| graph.degree_weighted(node))
        .collect::<Vec<_>>();
    let mut community_totals = degrees.clone();
    let mut nodes = (0..graph.len()).collect::<Vec<_>>();
    random.shuffle(&mut nodes);
    let modularity_denominator = 2.0 * total_weight.powi(2);
    // Community IDs remain dense graph-local indices throughout a Louvain
    // level. Reuse these arrays instead of allocating and hashing a temporary
    // map for every node visit.
    let mut community_positions = vec![usize::MAX; graph.len()];
    let mut weights = Vec::new();
    let mut improvement = false;
    loop {
        let mut moves = 0;
        for node in &nodes {
            let old_community = node_to_community[*node];
            let degree = degrees[*node];
            let old_weight_position = neighbor_community_weights(
                graph,
                *node,
                &node_to_community,
                old_community,
                &mut community_positions,
                &mut weights,
            );
            community_totals[old_community] -= degree;
            let old_weight = old_weight_position.map_or(0.0, |position| weights[position].1);
            let remove_cost = -old_weight / total_weight
                + resolution * (community_totals[old_community] * degree) / modularity_denominator;
            let mut best_gain = 0.0;
            let mut best_community = old_community;
            for &(community, weight) in &weights {
                let gain = remove_cost + weight / total_weight
                    - resolution * (community_totals[community] * degree) / modularity_denominator;
                if gain > best_gain {
                    best_gain = gain;
                    best_community = community;
                }
            }
            community_totals[best_community] += degree;
            if best_community != old_community {
                for member in &graph.members[*node] {
                    partition[old_community].remove(member);
                    partition[best_community].insert(member.clone());
                }
                inner[old_community].remove(node);
                inner[best_community].insert(*node);
                node_to_community[*node] = best_community;
                improvement = true;
                moves += 1;
            }
            for &(community, _) in &weights {
                community_positions[community] = usize::MAX;
            }
        }
        if moves == 0 {
            break;
        }
    }
    partition.retain(|community| !community.is_empty());
    inner.retain(|community| !community.is_empty());
    (partition, inner, improvement)
}

fn neighbor_community_weights(
    graph: &WeightedGraph,
    node: usize,
    node_to_community: &[usize],
    old_community: usize,
    community_positions: &mut [usize],
    output: &mut Vec<(usize, f64)>,
) -> Option<usize> {
    output.clear();
    let mut old_position = None;
    for (neighbor, weight) in &graph.adjacency[node] {
        if *neighbor == node {
            continue;
        }
        let community = node_to_community[*neighbor];
        let position = if community_positions[community] != usize::MAX {
            let position = community_positions[community];
            output[position].1 += weight;
            position
        } else {
            let position = output.len();
            community_positions[community] = position;
            output.push((community, *weight));
            position
        };
        if community == old_community {
            old_position = Some(position);
        }
    }
    old_position
}

fn modularity(graph: &WeightedGraph, communities: &[BTreeSet<usize>], resolution: f64) -> f64 {
    let degrees = (0..graph.len())
        .map(|node| graph.degree_weighted(node))
        .collect::<Vec<_>>();
    let degree_sum = degrees.iter().sum::<f64>();
    if degree_sum == 0.0 {
        return 0.0;
    }
    let total_weight = degree_sum / 2.0;
    let norm = 1.0 / degree_sum.powi(2);
    let mut node_community = vec![usize::MAX; graph.len()];
    let mut community_degrees = vec![0.0; communities.len()];
    let mut internal_weights = vec![0.0; communities.len()];
    for (community, nodes) in communities.iter().enumerate() {
        for node in nodes {
            node_community[*node] = community;
            community_degrees[community] += degrees[*node];
        }
    }
    for (left, right, weight) in graph.edges() {
        let community = node_community[left];
        if community != usize::MAX && node_community[right] == community {
            internal_weights[community] += weight;
        }
    }
    internal_weights
        .into_iter()
        .zip(community_degrees)
        .map(|(internal, degree)| internal / total_weight - resolution * degree.powi(2) * norm)
        .sum()
}

fn aggregate_graph(graph: &WeightedGraph, communities: &[BTreeSet<usize>]) -> WeightedGraph {
    let mut node_to_community = vec![0; graph.len()];
    let mut members = Vec::new();
    for (community, nodes) in communities.iter().enumerate() {
        let mut originals = BTreeSet::new();
        for node in nodes {
            node_to_community[*node] = community;
            originals.extend(graph.members[*node].iter().cloned());
        }
        members.push(originals);
    }
    let ids = (0..communities.len()).map(|id| id.to_string()).collect();
    let mut output = WeightedGraph::new(ids, members);
    for (left, right, weight) in graph.edges() {
        output.add_edge(node_to_community[left], node_to_community[right], weight);
    }
    output
}

#[derive(Clone)]
struct WeightedGraph {
    ids: Vec<String>,
    members: Vec<BTreeSet<String>>,
    adjacency: Vec<Vec<(usize, f64)>>,
}

impl WeightedGraph {
    fn new(ids: Vec<String>, members: Vec<BTreeSet<String>>) -> Self {
        let adjacency = vec![Vec::new(); ids.len()];
        Self {
            ids,
            members,
            adjacency,
        }
    }

    fn from_document(document: &GraphDocument) -> Self {
        let mut ids = document
            .nodes
            .iter()
            .map(|node| node.id.clone())
            .collect::<Vec<_>>();
        ids.sort();
        ids.dedup();
        let members = ids.iter().cloned().map(|id| BTreeSet::from([id])).collect();
        let positions = ids
            .iter()
            .enumerate()
            .map(|(index, id)| (id.clone(), index))
            .collect::<HashMap<_, _>>();
        let mut graph = Self::new(ids, members);
        let mut selected =
            HashMap::<(usize, usize), (usize, f64)>::with_capacity(document.links.len());
        for (edge_index, edge) in document.links.iter().enumerate() {
            let (Some(left), Some(right)) =
                (positions.get(&edge.source), positions.get(&edge.target))
            else {
                continue;
            };
            let weight = edge.number("weight").unwrap_or(1.0);
            let candidate = selected
                .entry((*left, *right))
                .or_insert((edge_index, weight));
            if candidate.1 != weight
                && canonical_edge_properties(edge)
                    >= canonical_edge_properties(&document.links[candidate.0])
            {
                *candidate = (edge_index, weight);
            }
        }
        let mut edges = selected
            .into_iter()
            .map(|((left, right), (_, weight))| (left, right, weight))
            .collect::<Vec<_>>();
        edges.sort_by_key(|(left, right, _)| (*left, *right));
        for (left, right, weight) in edges {
            graph.set_edge(left, right, weight);
        }
        graph
    }

    fn len(&self) -> usize {
        self.ids.len()
    }

    fn is_empty(&self) -> bool {
        self.ids.is_empty()
    }

    fn edge_count(&self) -> usize {
        self.edges().count()
    }

    fn position_map(&self) -> HashMap<&String, usize> {
        self.ids
            .iter()
            .enumerate()
            .map(|(index, id)| (id, index))
            .collect()
    }

    fn degree_unweighted(&self, node: usize) -> usize {
        self.adjacency[node]
            .iter()
            .map(|(neighbor, _)| if *neighbor == node { 2 } else { 1 })
            .sum()
    }

    fn degree_weighted(&self, node: usize) -> f64 {
        self.adjacency[node]
            .iter()
            .map(|(neighbor, weight)| {
                if *neighbor == node {
                    weight * 2.0
                } else {
                    *weight
                }
            })
            .sum()
    }

    fn total_weight(&self) -> f64 {
        self.edges().map(|(_, _, weight)| weight).sum()
    }

    fn set_edge(&mut self, left: usize, right: usize, weight: f64) {
        if let Some((_, existing)) = self.adjacency[left]
            .iter_mut()
            .find(|(neighbor, _)| *neighbor == right)
        {
            *existing = weight;
            if left != right
                && let Some((_, reverse)) = self.adjacency[right]
                    .iter_mut()
                    .find(|(neighbor, _)| *neighbor == left)
            {
                *reverse = weight;
            }
            return;
        }
        self.adjacency[left].push((right, weight));
        if left != right {
            self.adjacency[right].push((left, weight));
        }
    }

    fn add_edge(&mut self, left: usize, right: usize, weight: f64) {
        if let Some((_, existing)) = self.adjacency[left]
            .iter_mut()
            .find(|(neighbor, _)| *neighbor == right)
        {
            *existing += weight;
            if left != right
                && let Some((_, reverse)) = self.adjacency[right]
                    .iter_mut()
                    .find(|(neighbor, _)| *neighbor == left)
            {
                *reverse += weight;
            }
            return;
        }
        self.adjacency[left].push((right, weight));
        if left != right {
            self.adjacency[right].push((left, weight));
        }
    }

    fn edges(&self) -> impl Iterator<Item = (usize, usize, f64)> + '_ {
        self.adjacency
            .iter()
            .enumerate()
            .flat_map(|(left, neighbors)| {
                neighbors
                    .iter()
                    .filter(move |(right, _)| left <= *right)
                    .map(move |(right, weight)| (left, *right, *weight))
            })
    }

    fn subgraph(&self, selected: &[usize]) -> Self {
        let positions = selected
            .iter()
            .enumerate()
            .map(|(new, old)| (*old, new))
            .collect::<HashMap<_, _>>();
        let ids = selected
            .iter()
            .map(|node| self.ids[*node].clone())
            .collect();
        let members = selected
            .iter()
            .map(|node| self.members[*node].clone())
            .collect();
        let mut output = Self::new(ids, members);
        for (new_left, old_left) in selected.iter().enumerate() {
            for (old_right, weight) in &self.adjacency[*old_left] {
                if let Some(new_right) = positions.get(old_right) {
                    output.adjacency[new_left].push((*new_right, *weight));
                }
            }
        }
        output
    }
}

fn canonical_attributes(attributes: &serde_json::Map<String, Value>) -> String {
    fn canonical(value: &Value) -> Value {
        match value {
            Value::Object(map) => {
                let sorted = map
                    .iter()
                    .map(|(key, value)| (key.clone(), canonical(value)))
                    .collect::<BTreeMap<_, _>>();
                Value::Object(sorted.into_iter().collect())
            }
            Value::Array(values) => Value::Array(values.iter().map(canonical).collect()),
            other => other.clone(),
        }
    }
    serde_json::to_string(&canonical(&Value::Object(attributes.clone()))).unwrap_or_default()
}

fn canonical_edge_properties(edge: &EdgeRecord) -> String {
    let properties = edge
        .properties()
        .map(|(key, value)| (key.to_owned(), value))
        .collect::<serde_json::Map<_, _>>();
    canonical_attributes(&properties)
}

pub(crate) struct PythonRandom {
    state: [u32; 624],
    index: usize,
}

impl PythonRandom {
    pub(crate) fn seeded(seed: u32) -> Self {
        let mut random = Self {
            state: [0; 624],
            index: 624,
        };
        random.init_genrand(19_650_218);
        let key = [seed];
        let mut i = 1;
        let mut j = 0;
        for _ in 0..624 {
            let previous = random.state[i - 1];
            random.state[i] = (random.state[i]
                ^ (previous ^ (previous >> 30)).wrapping_mul(1_664_525))
            .wrapping_add(key[j])
            .wrapping_add(j as u32);
            i += 1;
            j += 1;
            if i >= 624 {
                random.state[0] = random.state[623];
                i = 1;
            }
            if j >= key.len() {
                j = 0;
            }
        }
        for _ in 0..623 {
            let previous = random.state[i - 1];
            random.state[i] = (random.state[i]
                ^ (previous ^ (previous >> 30)).wrapping_mul(1_566_083_941))
            .wrapping_sub(i as u32);
            i += 1;
            if i >= 624 {
                random.state[0] = random.state[623];
                i = 1;
            }
        }
        random.state[0] = 0x8000_0000;
        random
    }

    fn init_genrand(&mut self, seed: u32) {
        self.state[0] = seed;
        for index in 1..624 {
            self.state[index] = 1_812_433_253_u32
                .wrapping_mul(self.state[index - 1] ^ (self.state[index - 1] >> 30))
                .wrapping_add(index as u32);
        }
        self.index = 624;
    }

    fn next_u32(&mut self) -> u32 {
        if self.index >= 624 {
            for index in 0..624 {
                let value = (self.state[index] & 0x8000_0000)
                    | (self.state[(index + 1) % 624] & 0x7fff_ffff);
                self.state[index] = self.state[(index + 397) % 624]
                    ^ (value >> 1)
                    ^ if value & 1 == 0 { 0 } else { 0x9908_b0df };
            }
            self.index = 0;
        }
        let mut value = self.state[self.index];
        self.index += 1;
        value ^= value >> 11;
        value ^= (value << 7) & 0x9d2c_5680;
        value ^= (value << 15) & 0xefc6_0000;
        value ^ (value >> 18)
    }

    fn getrandbits(&mut self, bits: u32) -> u32 {
        self.next_u32() >> (32 - bits)
    }

    fn below(&mut self, upper: usize) -> usize {
        let bits = usize::BITS - upper.leading_zeros();
        loop {
            let value = self.getrandbits(bits) as usize;
            if value < upper {
                return value;
            }
        }
    }

    fn shuffle<T>(&mut self, values: &mut [T]) {
        for index in (1..values.len()).rev() {
            let replacement = self.below(index + 1);
            values.swap(index, replacement);
        }
    }

    pub(crate) fn sample_indices(&mut self, population: usize, count: usize) -> Vec<usize> {
        let mut result = Vec::with_capacity(count);
        let mut set_size = 21_usize;
        if count > 5 {
            let mut power = 1_usize;
            while power < count * 3 {
                power *= 4;
            }
            set_size += power;
        }
        if population <= set_size {
            let mut pool = (0..population).collect::<Vec<_>>();
            for index in 0..count {
                let selected = self.below(population - index);
                result.push(pool[selected]);
                pool[selected] = pool[population - index - 1];
            }
        } else {
            let mut selected = HashSet::new();
            for _ in 0..count {
                let mut candidate = self.below(population);
                while selected.contains(&candidate) {
                    candidate = self.below(population);
                }
                selected.insert(candidate);
                result.push(candidate);
            }
        }
        result
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use compass_model::{EdgeRecord, NodeRecord};
    use serde_json::{Map, json};

    fn graph(nodes: &[&str], edges: &[(&str, &str)]) -> GraphDocument {
        GraphDocument {
            directed: false,
            multigraph: false,
            graph: Map::new(),
            nodes: nodes
                .iter()
                .map(|id| NodeRecord {
                    id: (*id).to_owned(),
                    attributes: Map::from_iter([("label".to_owned(), json!(id))]),
                })
                .collect(),
            links: edges
                .iter()
                .map(|(source, target)| EdgeRecord {
                    source: (*source).to_owned(),
                    target: (*target).to_owned(),
                    attributes: Map::new(),
                })
                .collect(),
            extras: BTreeMap::new(),
        }
    }

    #[test]
    fn python_random_shuffle_matches_seed_42() {
        let mut values = (0..10).collect::<Vec<_>>();
        PythonRandom::seeded(42).shuffle(&mut values);
        assert_eq!(values, [7, 3, 2, 8, 5, 6, 9, 4, 0, 1]);
    }

    #[test]
    fn separates_two_dense_groups() {
        let document = graph(
            &["a", "b", "c", "x", "y", "z"],
            &[
                ("a", "b"),
                ("a", "c"),
                ("b", "c"),
                ("x", "y"),
                ("x", "z"),
                ("y", "z"),
                ("c", "x"),
            ],
        );
        assert_eq!(
            cluster(&document, ClusterOptions::default()),
            BTreeMap::from([
                (0, vec!["a".to_owned(), "b".to_owned(), "c".to_owned()]),
                (1, vec!["x".to_owned(), "y".to_owned(), "z".to_owned()]),
            ])
        );
    }

    #[test]
    fn incremental_clustering_freezes_unaffected_communities() {
        let mut document = graph(
            &["a", "b", "c", "x", "y", "z"],
            &[("a", "b"), ("b", "c"), ("x", "y"), ("y", "z")],
        );
        for node in &mut document.nodes {
            let source = if matches!(node.id.as_str(), "a" | "b" | "c") {
                "src/left.rs"
            } else {
                "src/right.rs"
            };
            node.attributes
                .insert("source_file".to_owned(), Value::String(source.to_owned()));
        }
        let previous = std::collections::HashMap::from([
            ("a".to_owned(), 7),
            ("b".to_owned(), 7),
            ("c".to_owned(), 7),
            ("x".to_owned(), 11),
            ("y".to_owned(), 11),
            ("z".to_owned(), 11),
        ]);

        let result = cluster_incremental(
            &document,
            &previous,
            &BTreeSet::from(["src/left.rs".to_owned()]),
            ClusterOptions::default(),
            IncrementalClusterLimits {
                max_affected_nodes: 4_096,
                max_affected_fraction: 0.75,
            },
        );

        assert!(result.used_incremental);
        assert!(result.affected_nodes < document.nodes.len());
        assert_eq!(
            result.communities.get(&11),
            Some(&vec!["x".to_owned(), "y".to_owned(), "z".to_owned()])
        );
        assert!(
            result.communities.values().any(|members| {
                members == &vec!["a".to_owned(), "b".to_owned(), "c".to_owned()]
            })
        );
    }

    #[test]
    fn incremental_clustering_falls_back_when_the_region_exceeds_its_bound() {
        let mut document = graph(&["a", "b", "c"], &[("a", "b"), ("b", "c")]);
        for node in &mut document.nodes {
            node.attributes.insert(
                "source_file".to_owned(),
                Value::String("src/all.rs".to_owned()),
            );
        }
        let previous = std::collections::HashMap::from([
            ("a".to_owned(), 0),
            ("b".to_owned(), 0),
            ("c".to_owned(), 0),
        ]);

        let result = cluster_incremental(
            &document,
            &previous,
            &BTreeSet::from(["src/all.rs".to_owned()]),
            ClusterOptions::default(),
            IncrementalClusterLimits {
                max_affected_nodes: 1,
                max_affected_fraction: 1.0,
            },
        );

        assert!(!result.used_incremental);
        assert_eq!(result.affected_nodes, document.nodes.len());
    }

    #[test]
    fn incremental_clustering_falls_back_for_a_removed_source() {
        let mut document = graph(&["a", "b"], &[("a", "b")]);
        for node in &mut document.nodes {
            node.attributes.insert(
                "source_file".to_owned(),
                Value::String("src/remaining.rs".to_owned()),
            );
        }
        let previous = std::collections::HashMap::from([
            ("a".to_owned(), 3),
            ("b".to_owned(), 3),
            ("deleted".to_owned(), 3),
        ]);

        let result = cluster_incremental(
            &document,
            &previous,
            &BTreeSet::from(["src/deleted.rs".to_owned()]),
            ClusterOptions::default(),
            IncrementalClusterLimits::default(),
        );

        assert!(!result.used_incremental);
        assert_eq!(result.affected_nodes, document.nodes.len());
    }

    #[test]
    fn incremental_clustering_falls_back_when_a_node_was_removed_elsewhere() {
        let mut document = graph(&["a", "b"], &[("a", "b")]);
        document.nodes[0].attributes.insert(
            "source_file".to_owned(),
            Value::String("src/changed.rs".to_owned()),
        );
        document.nodes[1].attributes.insert(
            "source_file".to_owned(),
            Value::String("src/remaining.rs".to_owned()),
        );
        let previous = std::collections::HashMap::from([
            ("a".to_owned(), 3),
            ("b".to_owned(), 4),
            ("deleted".to_owned(), 4),
        ]);

        let result = cluster_incremental(
            &document,
            &previous,
            &BTreeSet::from(["src/changed.rs".to_owned()]),
            ClusterOptions::default(),
            IncrementalClusterLimits::default(),
        );

        assert!(!result.used_incremental);
        assert_eq!(result.affected_nodes, document.nodes.len());
    }

    #[test]
    fn incremental_clustering_admits_a_boundary_community_as_a_whole() {
        let mut document = graph(
            &["a", "b", "x", "y", "z"],
            &[("a", "b"), ("b", "x"), ("x", "y"), ("y", "z")],
        );
        for node in &mut document.nodes {
            let source = if matches!(node.id.as_str(), "a" | "b") {
                "src/changed.rs"
            } else {
                "src/boundary.rs"
            };
            node.attributes
                .insert("source_file".to_owned(), Value::String(source.to_owned()));
        }
        let previous = std::collections::HashMap::from([
            ("a".to_owned(), 7),
            ("b".to_owned(), 7),
            ("x".to_owned(), 11),
            ("y".to_owned(), 11),
            ("z".to_owned(), 11),
        ]);

        let result = cluster_incremental(
            &document,
            &previous,
            &BTreeSet::from(["src/changed.rs".to_owned()]),
            ClusterOptions::default(),
            IncrementalClusterLimits {
                max_affected_nodes: document.nodes.len(),
                max_affected_fraction: 1.0,
            },
        );

        assert!(result.used_incremental);
        assert_eq!(result.affected_nodes, document.nodes.len());
    }

    #[test]
    fn empty_edgeless_and_split_graphs_have_total_deterministic_results() {
        assert!(cluster(&graph(&[], &[]), ClusterOptions::default()).is_empty());
        assert_eq!(
            cluster(&graph(&["b", "a"], &[]), ClusterOptions::default()),
            BTreeMap::from([(0, vec!["a".to_owned()]), (1, vec!["b".to_owned()])])
        );

        let weighted = WeightedGraph::from_document(&graph(&["b", "a"], &[]));
        assert!(excluded_hubs(&weighted, None).is_empty());
        assert!(excluded_hubs(&WeightedGraph::new(Vec::new(), Vec::new()), Some(50.0)).is_empty());
        assert_eq!(weighted.position_map().get(&&weighted.ids[0]), Some(&0));
        let positions = weighted.position_map();
        assert_eq!(
            split_community(&weighted, &positions, &["b".to_owned(), "a".to_owned()]),
            vec![vec!["a".to_owned()], vec!["b".to_owned()]]
        );
        assert_eq!(
            louvain(&weighted, 1.0),
            vec![vec!["a".to_owned()], vec!["b".to_owned()]]
        );
    }

    #[test]
    fn hub_reattachment_covers_connected_and_isolated_hubs() {
        let weighted =
            WeightedGraph::from_document(&graph(&["a", "hub", "isolated"], &[("hub", "a")]));
        let hub = weighted.ids.iter().position(|id| id == "hub").unwrap_or(0);
        let isolated = weighted
            .ids
            .iter()
            .position(|id| id == "isolated")
            .unwrap_or(0);
        let mut raw = vec![vec!["a".to_owned()]];
        reattach_hubs(&weighted, &HashSet::from([hub, isolated]), &mut raw);
        assert!(
            raw.iter()
                .any(|members| members.contains(&"hub".to_owned()))
        );
        assert!(
            raw.iter()
                .any(|members| members == &vec!["isolated".to_owned()])
        );
    }

    #[test]
    fn remapping_and_canonical_attributes_cover_unmatched_ties_and_nested_values() {
        assert!(
            remap_communities_to_previous(&Communities::new(), &std::collections::HashMap::new())
                .is_empty()
        );
        let communities = BTreeMap::from([
            (7, vec!["b".to_owned(), "a".to_owned()]),
            (4, vec!["c".to_owned(), "d".to_owned()]),
        ]);
        let remapped =
            remap_communities_to_previous(&communities, &std::collections::HashMap::new());
        assert_eq!(
            remapped.get(&0),
            Some(&vec!["a".to_owned(), "b".to_owned()])
        );
        assert_eq!(
            remapped.get(&1),
            Some(&vec!["c".to_owned(), "d".to_owned()])
        );

        let left = Map::from_iter([
            ("z".to_owned(), json!([{"b":2,"a":1}])),
            ("a".to_owned(), json!(true)),
        ]);
        let right = Map::from_iter([
            ("a".to_owned(), json!(true)),
            ("z".to_owned(), json!([{"a":1,"b":2}])),
        ]);
        assert_eq!(canonical_attributes(&left), canonical_attributes(&right));
    }

    #[test]
    fn linear_aggregations_match_the_reference_formulas() {
        let ids = ["a", "b", "c", "d"]
            .into_iter()
            .map(str::to_owned)
            .collect::<Vec<_>>();
        let members = ids.iter().cloned().map(|id| BTreeSet::from([id])).collect();
        let mut weighted = WeightedGraph::new(ids, members);
        weighted.add_edge(0, 1, 2.0);
        weighted.add_edge(1, 2, 1.5);
        weighted.add_edge(2, 3, 3.0);
        weighted.add_edge(3, 3, 0.5);
        let partition = [BTreeSet::from([0, 1]), BTreeSet::from([2, 3])];

        let degree_sum = (0..weighted.len())
            .map(|node| weighted.degree_weighted(node))
            .sum::<f64>();
        let total_weight = degree_sum / 2.0;
        let norm = 1.0 / degree_sum.powi(2);
        let reference = partition
            .iter()
            .map(|community| {
                let internal = weighted
                    .edges()
                    .filter(|(left, right, _)| {
                        community.contains(left) && community.contains(right)
                    })
                    .map(|(_, _, weight)| weight)
                    .sum::<f64>();
                let degree = community
                    .iter()
                    .map(|node| weighted.degree_weighted(*node))
                    .sum::<f64>();
                internal / total_weight - degree.powi(2) * norm
            })
            .sum::<f64>();
        assert!((modularity(&weighted, &partition, 1.0) - reference).abs() < 1e-12);

        let document = graph(&["a", "b", "c", "d"], &[("a", "b"), ("b", "c"), ("c", "d")]);
        let communities = BTreeMap::from([
            (0, vec!["a".to_owned(), "b".to_owned()]),
            (1, vec!["c".to_owned(), "d".to_owned()]),
        ]);
        let scores = score_communities(&document, &communities);
        for (community, nodes) in &communities {
            assert_eq!(
                scores.get(community),
                Some(&cohesion_score(&document, nodes))
            );
        }
    }

    #[test]
    fn python_random_sampling_covers_pool_and_rejection_algorithms() {
        let small = PythonRandom::seeded(7).sample_indices(10, 4);
        assert_eq!(small.len(), 4);
        assert_eq!(small.iter().copied().collect::<HashSet<_>>().len(), 4);
        let large = PythonRandom::seeded(7).sample_indices(1_000, 8);
        assert_eq!(large.len(), 8);
        assert_eq!(large.iter().copied().collect::<HashSet<_>>().len(), 8);
        assert!(large.iter().all(|index| *index < 1_000));
    }

    #[test]
    fn dangling_edges_are_ignored_when_building_weighted_graphs() {
        let mut document = graph(&["a"], &[]);
        document.links.push(EdgeRecord {
            source: "a".to_owned(),
            target: "missing".to_owned(),
            attributes: Map::from_iter([("weight".to_owned(), json!(3.0))]),
        });
        let weighted = WeightedGraph::from_document(&document);
        assert_eq!(weighted.len(), 1);
        assert_eq!(weighted.edge_count(), 0);
    }

    #[test]
    fn duplicate_hub_labels_add_source_context_only_when_needed() {
        let mut document = graph(&["a", "b", "c", "d", "unique"], &[("a", "b"), ("c", "d")]);
        for (index, file, line) in [
            (0, "crates/core/src/left.rs", 10),
            (2, "crates/core/src/right.rs", 20),
        ] {
            document.nodes[index]
                .attributes
                .insert("label".to_owned(), json!("shared"));
            document.nodes[index]
                .attributes
                .insert("source_file".to_owned(), json!(file));
            document.nodes[index]
                .attributes
                .insert("line_start".to_owned(), json!(line));
        }
        let communities = BTreeMap::from([
            (0, vec!["a".to_owned(), "b".to_owned()]),
            (1, vec!["c".to_owned(), "d".to_owned()]),
            (2, vec!["unique".to_owned()]),
        ]);

        assert_eq!(
            label_communities_by_hub(&document, &communities),
            BTreeMap::from([
                (0, "shared (src/left.rs:L10)".to_owned()),
                (1, "shared (src/right.rs:L20)".to_owned()),
                (2, "unique".to_owned()),
            ])
        );
    }

    #[test]
    fn duplicate_external_hubs_use_wiring_site_context() {
        let mut document = graph(&["a", "b", "c", "d"], &[("a", "b"), ("c", "d")]);
        for (index, file, line) in [
            (0, "python/tests/test_alter.py", 25),
            (2, "python/tests/test_writer.py", 81),
        ] {
            document.nodes[index]
                .attributes
                .insert("label".to_owned(), json!("DeltaTable"));
            document.nodes[index].attributes.insert(
                "evidence".to_owned(),
                json!([{"wiringSite": {"file": file, "startLine": line}}]),
            );
        }
        let communities = BTreeMap::from([
            (0, vec!["a".to_owned(), "b".to_owned()]),
            (1, vec!["c".to_owned(), "d".to_owned()]),
        ]);

        assert_eq!(
            label_communities_by_hub(&document, &communities),
            BTreeMap::from([
                (0, "DeltaTable (tests/test_alter.py:L25)".to_owned()),
                (1, "DeltaTable (tests/test_writer.py:L81)".to_owned()),
            ])
        );
    }

    #[test]
    fn identical_contexts_receive_deterministic_community_suffixes() {
        let mut document = graph(&["a", "b", "c", "d"], &[("a", "b"), ("c", "d")]);
        for index in [0, 2] {
            document.nodes[index]
                .attributes
                .insert("label".to_owned(), json!("shared"));
            document.nodes[index]
                .attributes
                .insert("source_file".to_owned(), json!("src/shared.rs"));
            document.nodes[index]
                .attributes
                .insert("line_start".to_owned(), json!(10));
        }
        let communities = BTreeMap::from([
            (7, vec!["a".to_owned(), "b".to_owned()]),
            (9, vec!["c".to_owned(), "d".to_owned()]),
        ]);

        let labels = label_communities_by_hub(&document, &communities);
        assert_eq!(
            labels.get(&7),
            Some(&"shared (src/shared.rs:L10) [community 7]".to_owned())
        );
        assert_eq!(
            labels.get(&9),
            Some(&"shared (src/shared.rs:L10) [community 9]".to_owned())
        );
        assert_eq!(labels.values().collect::<HashSet<_>>().len(), labels.len());
    }

    #[test]
    fn generated_label_collisions_from_untrusted_names_still_finish_unique() {
        let mut document = graph(&["a", "b", "c", "d", "e", "f"], &[("a", "b"), ("c", "d")]);
        for (index, label) in [
            (0, "shared"),
            (2, "shared"),
            (4, "shared (src/left.rs:L10)"),
            (5, "shared (src/left.rs:L10) [community 0]"),
        ] {
            document.nodes[index]
                .attributes
                .insert("label".to_owned(), json!(label));
        }
        for (index, file, line) in [
            (0, "crates/core/src/left.rs", 10),
            (2, "crates/core/src/right.rs", 20),
        ] {
            document.nodes[index]
                .attributes
                .insert("source_file".to_owned(), json!(file));
            document.nodes[index]
                .attributes
                .insert("line_start".to_owned(), json!(line));
        }
        let communities = BTreeMap::from([
            (0, vec!["a".to_owned(), "b".to_owned()]),
            (1, vec!["c".to_owned(), "d".to_owned()]),
            (2, vec!["e".to_owned()]),
            (3, vec!["f".to_owned()]),
        ]);

        let labels = label_communities_by_hub(&document, &communities);
        assert_eq!(labels.values().collect::<HashSet<_>>().len(), labels.len());
        for (community, label) in labels {
            assert!(label.ends_with(&format!("[community {community}]")));
        }
    }
}
