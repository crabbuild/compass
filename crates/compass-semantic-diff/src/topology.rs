use std::collections::{BTreeMap, BTreeSet};

use compass_model::code_graph::GraphDocument;

use crate::{MAX_TRAVERSED_CALL_EDGES, SemanticDiffError};

/// Bounded strongly connected components for dependency-cycle queries.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DependencyCycleIndex {
    positions: BTreeMap<String, usize>,
    components: Vec<usize>,
}

impl DependencyCycleIndex {
    /// Build one deterministic index under the semantic traversal work bound.
    pub fn from_graph(document: &GraphDocument) -> Result<Self, SemanticDiffError> {
        let mut node_ids = BTreeSet::new();
        let mut edges = Vec::new();
        for edge in &document.links {
            if edge.deferred || !is_dependency_relation(edge.relation()) {
                continue;
            }
            if edges.len() >= MAX_TRAVERSED_CALL_EDGES {
                return Err(SemanticDiffError::LimitExceeded {
                    resource: "dependency_cycle_edges",
                    limit: MAX_TRAVERSED_CALL_EDGES,
                });
            }
            node_ids.insert(edge.semantic_source().to_owned());
            node_ids.insert(edge.semantic_target().to_owned());
            edges.push((
                edge.semantic_source().to_owned(),
                edge.semantic_target().to_owned(),
            ));
        }
        if node_ids.len() > MAX_TRAVERSED_CALL_EDGES {
            return Err(SemanticDiffError::LimitExceeded {
                resource: "dependency_cycle_nodes",
                limit: MAX_TRAVERSED_CALL_EDGES,
            });
        }
        let positions = node_ids
            .into_iter()
            .enumerate()
            .map(|(position, id)| (id, position))
            .collect::<BTreeMap<_, _>>();
        let mut forward = vec![Vec::new(); positions.len()];
        let mut reverse = vec![Vec::new(); positions.len()];
        for (source, target) in edges {
            let left = positions[&source];
            let right = positions[&target];
            forward[left].push(right);
            reverse[right].push(left);
        }
        normalize_adjacency(&mut forward);
        normalize_adjacency(&mut reverse);
        let order = finishing_order(&forward);
        let components = assign_components(&reverse, &order);
        Ok(Self {
            positions,
            components,
        })
    }

    /// Return true when the exact changed edge belongs to a directed cycle.
    #[must_use]
    pub fn participates_in_cycle(&self, source: &str, target: &str) -> bool {
        if source == target {
            return self.positions.contains_key(source);
        }
        match (self.positions.get(source), self.positions.get(target)) {
            (Some(source), Some(target)) => self.components[*source] == self.components[*target],
            _ => false,
        }
    }
}

/// Prove whether `source -> target` closes a directed dependency cycle.
pub fn dependency_participates_in_cycle(
    document: &GraphDocument,
    source: &str,
    target: &str,
) -> Result<bool, SemanticDiffError> {
    Ok(DependencyCycleIndex::from_graph(document)?.participates_in_cycle(source, target))
}

fn normalize_adjacency(adjacency: &mut [Vec<usize>]) {
    for neighbors in adjacency {
        neighbors.sort_unstable();
        neighbors.dedup();
    }
}

fn finishing_order(adjacency: &[Vec<usize>]) -> Vec<usize> {
    let mut visited = vec![false; adjacency.len()];
    let mut order = Vec::with_capacity(adjacency.len());
    for start in 0..adjacency.len() {
        if visited[start] {
            continue;
        }
        visited[start] = true;
        let mut stack = vec![(start, 0_usize)];
        while let Some((node, next)) = stack.last_mut() {
            if let Some(&neighbor) = adjacency[*node].get(*next) {
                *next += 1;
                if !visited[neighbor] {
                    visited[neighbor] = true;
                    stack.push((neighbor, 0));
                }
            } else if let Some((finished, _)) = stack.pop() {
                order.push(finished);
            }
        }
    }
    order
}

fn assign_components(reverse: &[Vec<usize>], order: &[usize]) -> Vec<usize> {
    let mut components = vec![usize::MAX; reverse.len()];
    let mut component = 0_usize;
    for &start in order.iter().rev() {
        if components[start] != usize::MAX {
            continue;
        }
        components[start] = component;
        let mut stack = vec![start];
        while let Some(node) = stack.pop() {
            for &neighbor in &reverse[node] {
                if components[neighbor] == usize::MAX {
                    components[neighbor] = component;
                    stack.push(neighbor);
                }
            }
        }
        component = component.saturating_add(1);
    }
    components
}

fn is_dependency_relation(relation: &str) -> bool {
    matches!(
        relation,
        "calls" | "imports" | "imports_from" | "depends_on" | "uses" | "references"
    )
}

#[cfg(test)]
mod tests {
    use compass_model::code_graph::{BuildMetadata, EdgeKind, EdgeRecord, GraphDocument};

    use super::dependency_participates_in_cycle;

    #[test]
    fn cycle_evidence_respects_direction_and_deferred_edges() -> Result<(), crate::SemanticDiffError>
    {
        let mut graph = GraphDocument::empty_v1(BuildMetadata {
            builder_version: "test".to_owned(),
            schema_fingerprint: "schema".to_owned(),
            source_tree_digest: "tree".to_owned(),
            configuration_digest: "config".to_owned(),
            generation_id: "generation".to_owned(),
            source_commit: None,
        });
        graph.links = vec![
            edge("a-b", "a", "b", false),
            edge("b-c", "b", "c", false),
            edge("c-a", "c", "a", false),
        ];
        assert!(dependency_participates_in_cycle(&graph, "a", "b")?);
        assert!(!dependency_participates_in_cycle(&graph, "a", "d")?);

        graph.links[2].deferred = true;
        assert!(!dependency_participates_in_cycle(&graph, "a", "b")?);
        graph.links.push(edge("self", "self", "self", false));
        assert!(dependency_participates_in_cycle(&graph, "self", "self")?);
        Ok(())
    }

    fn edge(id: &str, source: &str, target: &str, deferred: bool) -> EdgeRecord {
        EdgeRecord {
            id: id.to_owned(),
            key: id.to_owned(),
            source: source.to_owned(),
            target: target.to_owned(),
            kind: EdgeKind::Calls,
            occurrence_rule: None,
            relationship_site: None,
            details: None,
            evidence: Vec::new(),
            weight: None,
            context: None,
            deferred,
            diagnostics: Vec::new(),
        }
    }
}
