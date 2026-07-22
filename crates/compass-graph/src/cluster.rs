use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};

use compass_model::GraphDocument;
use serde_json::Value;
use sha2::{Digest, Sha256};

const MAX_COMMUNITY_FRACTION: f64 = 0.25;
const MIN_SPLIT_SIZE: usize = 10;
const COHESION_SPLIT_THRESHOLD: f64 = 0.05;
const COHESION_SPLIT_MIN_SIZE: usize = 50;
const LOUVAIN_THRESHOLD: f64 = 1e-4;
const LOUVAIN_MAX_LEVEL: usize = 10;

pub type Communities = BTreeMap<usize, Vec<String>>;

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
    let graph = WeightedGraph::from_document(document);
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

    let mut raw = Vec::<Vec<String>>::new();
    if !connected.is_empty() {
        raw.extend(louvain(&connected, options.resolution));
    }
    raw.extend(
        isolates
            .into_iter()
            .map(|node| vec![graph.ids[node].clone()]),
    );

    if !hubs.is_empty() {
        reattach_hubs(&graph, &hubs, &mut raw);
    }

    let maximum_size = MIN_SPLIT_SIZE.max((graph.len() as f64 * MAX_COMMUNITY_FRACTION) as usize);
    let mut first_pass = Vec::new();
    for members in raw {
        if members.len() > maximum_size {
            first_pass.extend(split_community(&graph, &members));
        } else {
            first_pass.push(members);
        }
    }
    let mut final_communities = Vec::new();
    for members in first_pass {
        if members.len() >= COHESION_SPLIT_MIN_SIZE
            && cohesion_score_graph(&graph, &members) < COHESION_SPLIT_THRESHOLD
        {
            let splits = split_community(&graph, &members);
            if splits.len() > 1 {
                final_communities.extend(splits);
            } else {
                final_communities.push(members);
            }
        } else {
            final_communities.push(members);
        }
    }
    for members in &mut final_communities {
        members.sort();
    }
    final_communities
        .sort_by(|left, right| right.len().cmp(&left.len()).then_with(|| left.cmp(right)));
    final_communities.into_iter().enumerate().collect()
}

#[must_use]
pub fn label_communities_by_hub(
    document: &GraphDocument,
    communities: &Communities,
) -> BTreeMap<usize, String> {
    let graph = WeightedGraph::from_document(document);
    let positions = graph
        .ids
        .iter()
        .enumerate()
        .map(|(index, id)| (id.as_str(), index))
        .collect::<HashMap<_, _>>();
    let labels = document
        .nodes
        .iter()
        .map(|node| (node.id.as_str(), node.label()))
        .collect::<HashMap<_, _>>();
    let mut output = BTreeMap::new();
    for (community, members) in communities {
        let hub = members
            .iter()
            .filter_map(|member| positions.get(member.as_str()).map(|index| (member, *index)))
            .min_by(|(left_id, left), (right_id, right)| {
                graph
                    .degree_unweighted(*right)
                    .cmp(&graph.degree_unweighted(*left))
                    .then_with(|| left_id.cmp(right_id))
            });
        let fallback = format!("Community {community}");
        let name = hub
            .and_then(|(id, _)| labels.get(id.as_str()).copied().or(Some(id.as_str())))
            .map(str::trim)
            .map(|value| value.strip_suffix("()").unwrap_or(value))
            .filter(|value| !value.is_empty())
            .unwrap_or(&fallback)
            .to_owned();
        output.insert(*community, name);
    }
    output
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
    cohesion_score_graph(&WeightedGraph::from_document(document), members)
}

#[must_use]
pub fn score_communities(
    document: &GraphDocument,
    communities: &Communities,
) -> BTreeMap<usize, f64> {
    let graph = WeightedGraph::from_document(document);
    communities
        .iter()
        .map(|(community, members)| (*community, cohesion_score_graph(&graph, members)))
        .collect()
}

#[must_use]
pub fn remap_communities_to_previous(
    communities: &Communities,
    previous: &HashMap<String, usize>,
) -> Communities {
    if communities.is_empty() {
        return Communities::new();
    }
    let new_sets = communities
        .iter()
        .map(|(community, nodes)| (*community, nodes.iter().collect::<HashSet<_>>()))
        .collect::<HashMap<_, _>>();
    let mut old_sets = HashMap::<usize, HashSet<&String>>::new();
    for (node, community) in previous {
        old_sets.entry(*community).or_default().insert(node);
    }
    let mut overlaps = Vec::<(usize, usize, usize)>::new();
    for (old_community, old_nodes) in old_sets {
        for (new_community, new_nodes) in &new_sets {
            let overlap = old_nodes.intersection(new_nodes).count();
            if overlap > 0 {
                overlaps.push((overlap, old_community, *new_community));
            }
        }
    }
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

fn split_community(graph: &WeightedGraph, members: &[String]) -> Vec<Vec<String>> {
    let positions = graph.position_map();
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

fn cohesion_score_graph(graph: &WeightedGraph, members: &[String]) -> f64 {
    let count = members.len();
    if count <= 1 {
        return 1.0;
    }
    let member_set = members.iter().map(String::as_str).collect::<HashSet<_>>();
    let actual = graph
        .edges()
        .iter()
        .filter(|(left, right, _)| {
            member_set.contains(graph.ids[*left].as_str())
                && member_set.contains(graph.ids[*right].as_str())
        })
        .count();
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
    let mut improvement = false;
    loop {
        let mut moves = 0;
        for node in &nodes {
            let old_community = node_to_community[*node];
            let degree = degrees[*node];
            let weights = neighbor_community_weights(graph, *node, &node_to_community);
            community_totals[old_community] -= degree;
            let remove_cost = -weight_for(&weights, old_community) / total_weight
                + resolution * (community_totals[old_community] * degree)
                    / (2.0 * total_weight.powi(2));
            let mut best_gain = 0.0;
            let mut best_community = old_community;
            for (community, weight) in weights {
                let gain = remove_cost + weight / total_weight
                    - resolution * (community_totals[community] * degree)
                        / (2.0 * total_weight.powi(2));
                if gain > best_gain {
                    best_gain = gain;
                    best_community = community;
                }
            }
            community_totals[best_community] += degree;
            if best_community != old_community {
                let original_members = graph.members[*node].clone();
                for member in &original_members {
                    partition[old_community].remove(member);
                    partition[best_community].insert(member.clone());
                }
                inner[old_community].remove(node);
                inner[best_community].insert(*node);
                node_to_community[*node] = best_community;
                improvement = true;
                moves += 1;
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
) -> Vec<(usize, f64)> {
    let mut output = Vec::<(usize, f64)>::new();
    let mut positions = HashMap::<usize, usize>::new();
    for (neighbor, weight) in &graph.adjacency[node] {
        if *neighbor == node {
            continue;
        }
        let community = node_to_community[*neighbor];
        if let Some(position) = positions.get(&community) {
            output[*position].1 += weight;
        } else {
            positions.insert(community, output.len());
            output.push((community, *weight));
        }
    }
    output
}

fn weight_for(weights: &[(usize, f64)], community: usize) -> f64 {
    weights
        .iter()
        .find(|(candidate, _)| *candidate == community)
        .map_or(0.0, |(_, weight)| *weight)
}

fn modularity(graph: &WeightedGraph, communities: &[BTreeSet<usize>], resolution: f64) -> f64 {
    let degree_sum = (0..graph.len())
        .map(|node| graph.degree_weighted(node))
        .sum::<f64>();
    let total_weight = degree_sum / 2.0;
    let norm = 1.0 / degree_sum.powi(2);
    communities
        .iter()
        .map(|community| {
            let internal = graph
                .edges()
                .iter()
                .filter(|(left, right, _)| community.contains(left) && community.contains(right))
                .map(|(_, _, weight)| *weight)
                .sum::<f64>();
            let degree = community
                .iter()
                .map(|node| graph.degree_weighted(*node))
                .sum::<f64>();
            internal / total_weight - resolution * degree.powi(2) * norm
        })
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
        let mut edges = document.links.iter().collect::<Vec<_>>();
        edges.sort_by(|left, right| {
            left.source
                .cmp(&right.source)
                .then_with(|| left.target.cmp(&right.target))
                .then_with(|| {
                    canonical_attributes(&left.attributes)
                        .cmp(&canonical_attributes(&right.attributes))
                })
        });
        for edge in edges {
            let (Some(left), Some(right)) =
                (positions.get(&edge.source), positions.get(&edge.target))
            else {
                continue;
            };
            let weight = edge
                .attributes
                .get("weight")
                .and_then(Value::as_f64)
                .unwrap_or(1.0);
            graph.set_edge(*left, *right, weight);
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
        self.edges().len()
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
        self.edges().iter().map(|(_, _, weight)| *weight).sum()
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

    fn edges(&self) -> Vec<(usize, usize, f64)> {
        let mut output = Vec::new();
        let mut visited = HashSet::new();
        for left in 0..self.len() {
            for (right, weight) in &self.adjacency[left] {
                if !visited.contains(right) {
                    output.push((left, *right, *weight));
                }
            }
            visited.insert(left);
        }
        output
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
            used_legacy_edges_key: false,
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
        assert_eq!(
            split_community(&weighted, &["b".to_owned(), "a".to_owned()]),
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
        assert!(remap_communities_to_previous(&Communities::new(), &HashMap::new()).is_empty());
        let communities = BTreeMap::from([
            (7, vec!["b".to_owned(), "a".to_owned()]),
            (4, vec!["c".to_owned(), "d".to_owned()]),
        ]);
        let remapped = remap_communities_to_previous(&communities, &HashMap::new());
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
}
