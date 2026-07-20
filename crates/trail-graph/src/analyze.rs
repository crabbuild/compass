use std::collections::{BTreeMap, HashMap, HashSet, VecDeque};
use std::path::Path;

use serde::Serialize;
use serde_json::Value;
use trail_model::{EdgeRecord, GraphDocument, NodeRecord};

use crate::cluster::{Communities, PythonRandom, cohesion_score};

const BUILTIN_NOISE_LABELS: &[&str] = &[
    "str",
    "int",
    "float",
    "bool",
    "bytes",
    "bytearray",
    "complex",
    "object",
    "True",
    "False",
    "MagicMock",
    "Mock",
    "AsyncMock",
    "NonCallableMock",
    "NonCallableMagicMock",
    "PropertyMock",
    "patch",
    "sentinel",
    "Path",
    "Any",
    "Optional",
    "List",
    "Dict",
    "Set",
    "Tuple",
    "Union",
    "Callable",
    "Type",
    "ClassVar",
    "Final",
    "Literal",
    "Protocol",
    "Counter",
    "defaultdict",
    "OrderedDict",
    "datetime",
    "Enum",
    "os",
    "sys",
    "re",
    "json",
    "io",
    "abc",
    "typing",
];

const JSON_NOISE_LABELS: &[&str] = &[
    "start",
    "end",
    "name",
    "id",
    "type",
    "properties",
    "value",
    "key",
    "data",
    "items",
    "title",
    "description",
    "version",
    "dependencies",
    "devdependencies",
    "peerdependencies",
    "optionaldependencies",
    "bundleddependencies",
    "bundledependencies",
];

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct GodNode {
    pub id: String,
    pub label: String,
    pub degree: usize,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct SurpriseConnection {
    pub source: String,
    pub target: String,
    pub source_files: [String; 2],
    pub confidence: String,
    pub relation: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub why: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct SuggestedQuestion {
    #[serde(rename = "type")]
    pub kind: String,
    pub question: Option<String>,
    pub why: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct DiffNode {
    pub id: String,
    pub label: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct DiffEdge {
    pub source: String,
    pub target: String,
    pub relation: String,
    pub confidence: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct GraphDiff {
    pub new_nodes: Vec<DiffNode>,
    pub removed_nodes: Vec<DiffNode>,
    pub new_edges: Vec<DiffEdge>,
    pub removed_edges: Vec<DiffEdge>,
    pub summary: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct ImportCycle {
    pub cycle: Vec<String>,
    pub length: usize,
    pub why: String,
}

#[must_use]
pub fn god_nodes(document: &GraphDocument, top_n: usize) -> Vec<GodNode> {
    let graph = AnalysisGraph::new(document);
    let mut ranked = graph
        .nodes
        .iter()
        .enumerate()
        .map(|(position, node)| (position, node, graph.degree(position)))
        .collect::<Vec<_>>();
    ranked.sort_by(|left, right| right.2.cmp(&left.2).then_with(|| left.0.cmp(&right.0)));
    ranked
        .into_iter()
        .filter(|(position, node, _)| {
            !graph.is_file_node(*position)
                && !is_concept_node(node)
                && !is_json_key_node(node)
                && !BUILTIN_NOISE_LABELS.contains(&node.label())
        })
        .take(top_n)
        .map(|(_, node, degree)| GodNode {
            id: node.id.clone(),
            label: node.label().to_owned(),
            degree,
        })
        .collect()
}

#[must_use]
pub fn surprising_connections(
    document: &GraphDocument,
    communities: &Communities,
    top_n: usize,
) -> Vec<SurpriseConnection> {
    let graph = AnalysisGraph::new(document);
    let source_count = graph
        .nodes
        .iter()
        .filter_map(|node| attribute(node, "source_file"))
        .filter(|source| !source.is_empty())
        .collect::<HashSet<_>>()
        .len();
    if source_count > 1 {
        let cross_file = cross_file_surprises(&graph, communities, top_n);
        if !cross_file.is_empty() {
            return cross_file;
        }
    }
    cross_community_surprises(&graph, communities, top_n)
}

#[must_use]
pub fn suggest_questions(
    document: &GraphDocument,
    communities: &Communities,
    community_labels: &BTreeMap<usize, String>,
    top_n: usize,
) -> Vec<SuggestedQuestion> {
    let graph = AnalysisGraph::new(document);
    let node_community = invert_communities(communities);
    let mut questions = Vec::new();
    for edge in &graph.edges {
        if edge_string(edge.record, "confidence") != "AMBIGUOUS" {
            continue;
        }
        let left = &graph.nodes[edge.left];
        let right = &graph.nodes[edge.right];
        let relation = edge_string(edge.record, "relation");
        let relation = if relation.is_empty() {
            "related to".to_owned()
        } else {
            relation
        };
        questions.push(SuggestedQuestion {
            kind: "ambiguous_edge".to_owned(),
            question: Some(format!(
                "What is the exact relationship between `{}` and `{}`?",
                left.label(),
                right.label()
            )),
            why: format!("Edge tagged AMBIGUOUS (relation: {relation}) - confidence is low."),
        });
    }

    if !graph.edges.is_empty() {
        let centrality = node_betweenness(&graph, graph.len() > 1000);
        let mut bridges = centrality
            .iter()
            .enumerate()
            .filter(|(node, score)| {
                **score > 0.0 && !graph.is_file_node(*node) && !is_concept_node(graph.nodes[*node])
            })
            .collect::<Vec<_>>();
        bridges.sort_by(|left, right| {
            right
                .1
                .partial_cmp(left.1)
                .unwrap_or(std::cmp::Ordering::Equal)
                .then_with(|| left.0.cmp(&right.0))
        });
        for (node, score) in bridges.into_iter().take(3) {
            let id = &graph.nodes[node].id;
            let community = node_community.get(id).copied();
            let community_label = community
                .and_then(|cid| community_labels.get(&cid).cloned())
                .or_else(|| community.map(|cid| format!("Community {cid}")))
                .unwrap_or_else(|| "unknown".to_owned());
            let mut other_communities = Vec::new();
            let mut seen = HashSet::new();
            for neighbor in &graph.adjacency[node] {
                let neighbor_community = node_community.get(&graph.nodes[*neighbor].id).copied();
                if neighbor_community != community && seen.insert(neighbor_community) {
                    other_communities.push(neighbor_community);
                }
            }
            if other_communities.is_empty() {
                continue;
            }
            let other_labels = other_communities
                .into_iter()
                .map(|candidate| {
                    candidate
                        .and_then(|cid| community_labels.get(&cid).cloned())
                        .or_else(|| candidate.map(|cid| format!("Community {cid}")))
                        .unwrap_or_else(|| "Community None".to_owned())
                })
                .map(|label| format!("`{label}`"))
                .collect::<Vec<_>>()
                .join(", ");
            questions.push(SuggestedQuestion {
                kind: "bridge_node".to_owned(),
                question: Some(format!(
                    "Why does `{}` connect `{community_label}` to {other_labels}?",
                    graph.nodes[node].label()
                )),
                why: format!(
                    "High betweenness centrality ({score:.3}) - this node is a cross-community bridge."
                ),
            });
        }
    }

    let mut ranked = (0..graph.len()).collect::<Vec<_>>();
    ranked.sort_by_key(|node| (std::cmp::Reverse(graph.degree(*node)), *node));
    for node in ranked
        .into_iter()
        .filter(|node| !graph.is_file_node(*node))
        .take(5)
    {
        let inferred = graph
            .incident_edges(node)
            .into_iter()
            .filter(|edge| edge_string(edge.record, "confidence") == "INFERRED")
            .collect::<Vec<_>>();
        if inferred.len() < 2 {
            continue;
        }
        let others = inferred
            .iter()
            .take(2)
            .map(|edge| {
                let other = oriented_other(&graph, edge, node);
                graph.nodes[other].label().to_owned()
            })
            .collect::<Vec<_>>();
        let label = graph.nodes[node].label();
        questions.push(SuggestedQuestion {
            kind: "verify_inferred".to_owned(),
            question: Some(format!(
                "Are the {} inferred relationships involving `{label}` (e.g. with `{}` and `{}`) actually correct?",
                inferred.len(), others[0], others[1]
            )),
            why: format!(
                "`{label}` has {} INFERRED edges - model-reasoned connections that need verification.",
                inferred.len()
            ),
        });
    }

    let isolated = (0..graph.len())
        .filter(|node| {
            graph.degree(*node) <= 1
                && !graph.is_file_node(*node)
                && !is_concept_node(graph.nodes[*node])
                && attribute(graph.nodes[*node], "file_type") != Some("rationale")
        })
        .collect::<Vec<_>>();
    if !isolated.is_empty() {
        let labels = isolated
            .iter()
            .take(3)
            .map(|node| format!("`{}`", graph.nodes[*node].label()))
            .collect::<Vec<_>>()
            .join(", ");
        questions.push(SuggestedQuestion {
            kind: "isolated_nodes".to_owned(),
            question: Some(format!("What connects {labels} to the rest of the system?")),
            why: format!(
                "{} weakly-connected nodes found - possible documentation gaps or missing edges.",
                isolated.len()
            ),
        });
    }
    for (community, members) in communities {
        let score = cohesion_score(document, members);
        if score < 0.15 && members.len() >= 5 {
            let label = community_labels
                .get(community)
                .cloned()
                .unwrap_or_else(|| format!("Community {community}"));
            questions.push(SuggestedQuestion {
                kind: "low_cohesion".to_owned(),
                question: Some(format!(
                    "Should `{label}` be split into smaller, more focused modules?"
                )),
                why: format!(
                    "Cohesion score {score} - nodes in this community are weakly interconnected."
                ),
            });
        }
    }
    if questions.is_empty() {
        questions.push(SuggestedQuestion {
            kind: "no_signal".to_owned(),
            question: None,
            why: "Not enough signal to generate questions. This usually means the corpus has no AMBIGUOUS edges, no bridge nodes, no INFERRED relationships, and all communities are tightly cohesive. Add more files or run with --mode deep to extract richer edges.".to_owned(),
        });
    }
    questions.truncate(top_n);
    questions
}

#[must_use]
pub fn graph_diff(old: &GraphDocument, new: &GraphDocument) -> GraphDiff {
    let old_nodes = old
        .nodes
        .iter()
        .map(|node| node.id.as_str())
        .collect::<HashSet<_>>();
    let new_nodes = new
        .nodes
        .iter()
        .map(|node| node.id.as_str())
        .collect::<HashSet<_>>();
    let new_nodes_list = new
        .nodes
        .iter()
        .filter(|node| !old_nodes.contains(node.id.as_str()))
        .map(|node| DiffNode {
            id: node.id.clone(),
            label: node.label().to_owned(),
        })
        .collect::<Vec<_>>();
    let removed_nodes_list = old
        .nodes
        .iter()
        .filter(|node| !new_nodes.contains(node.id.as_str()))
        .map(|node| DiffNode {
            id: node.id.clone(),
            label: node.label().to_owned(),
        })
        .collect::<Vec<_>>();
    let old_keys = old
        .links
        .iter()
        .map(|edge| diff_edge_key(old.directed, edge))
        .collect::<HashSet<_>>();
    let new_keys = new
        .links
        .iter()
        .map(|edge| diff_edge_key(new.directed, edge))
        .collect::<HashSet<_>>();
    let new_edges = new
        .links
        .iter()
        .filter(|edge| !old_keys.contains(&diff_edge_key(new.directed, edge)))
        .map(diff_edge)
        .collect::<Vec<_>>();
    let removed_edges = old
        .links
        .iter()
        .filter(|edge| !new_keys.contains(&diff_edge_key(old.directed, edge)))
        .map(diff_edge)
        .collect::<Vec<_>>();
    let mut parts = Vec::new();
    if !new_nodes_list.is_empty() {
        parts.push(plural(new_nodes_list.len(), "new node", "new nodes"));
    }
    if !new_edges.is_empty() {
        parts.push(plural(new_edges.len(), "new edge", "new edges"));
    }
    if !removed_nodes_list.is_empty() {
        parts.push(plural(
            removed_nodes_list.len(),
            "node removed",
            "nodes removed",
        ));
    }
    if !removed_edges.is_empty() {
        parts.push(plural(removed_edges.len(), "edge removed", "edges removed"));
    }
    GraphDiff {
        new_nodes: new_nodes_list,
        removed_nodes: removed_nodes_list,
        new_edges,
        removed_edges,
        summary: if parts.is_empty() {
            "no changes".to_owned()
        } else {
            parts.join(", ")
        },
    }
}

#[must_use]
pub fn find_import_cycles(
    document: &GraphDocument,
    max_cycle_length: usize,
    top_n: usize,
) -> Vec<ImportCycle> {
    let graph = AnalysisGraph::new(document);
    let mut files = Vec::<String>::new();
    let mut file_position = HashMap::<String, usize>::new();
    let mut arcs = Vec::<(usize, usize)>::new();
    for edge in &graph.edges {
        let relation = edge_string(edge.record, "relation");
        if !matches!(relation.as_str(), "imports_from" | "re_exports")
            || edge
                .record
                .attributes
                .get("deferred")
                .is_some_and(Value::is_boolean_and_true)
        {
            continue;
        }
        let source_file = edge_string(edge.record, "source_file");
        if source_file.is_empty() {
            continue;
        }
        let left_file = attribute(graph.nodes[edge.left], "source_file").unwrap_or_default();
        let right_file = attribute(graph.nodes[edge.right], "source_file").unwrap_or_default();
        let target_file = if left_file == source_file {
            right_file
        } else if right_file == source_file {
            left_file
        } else if !right_file.is_empty() && right_file != source_file {
            right_file
        } else {
            left_file
        };
        if target_file.is_empty() {
            continue;
        }
        let left = file_index(&source_file, &mut files, &mut file_position);
        let right = file_index(target_file, &mut files, &mut file_position);
        if !arcs.contains(&(left, right)) {
            arcs.push((left, right));
        }
    }
    if arcs.is_empty() {
        return Vec::new();
    }
    let mut adjacency = vec![Vec::new(); files.len()];
    for (left, right) in arcs {
        adjacency[left].push(right);
    }
    let mut cycles = Vec::<Vec<usize>>::new();
    for start in 0..files.len() {
        let mut path = vec![start];
        let mut visited = HashSet::from([start]);
        enumerate_cycles(
            start,
            start,
            &adjacency,
            max_cycle_length,
            &mut path,
            &mut visited,
            &mut cycles,
            top_n * 10,
        );
        if cycles.len() >= top_n * 10 {
            break;
        }
    }
    cycles.sort_by_key(Vec::len);
    let mut seen = HashSet::new();
    let mut output = Vec::new();
    for cycle in cycles {
        let mut names = cycle
            .into_iter()
            .map(|node| files[node].clone())
            .collect::<Vec<_>>();
        let minimum = names
            .iter()
            .enumerate()
            .min_by_key(|(_, name)| *name)
            .map_or(0, |(index, _)| index);
        names.rotate_left(minimum);
        if seen.insert(names.clone()) {
            output.push(ImportCycle {
                length: names.len(),
                cycle: names,
                why: "circular dependency".to_owned(),
            });
            if output.len() >= top_n {
                break;
            }
        }
    }
    output
}

fn cross_file_surprises(
    graph: &AnalysisGraph<'_>,
    communities: &Communities,
    top_n: usize,
) -> Vec<SurpriseConnection> {
    let node_community = invert_communities(communities);
    let mut candidates = Vec::<(i32, usize, SurpriseConnection)>::new();
    for (position, edge) in graph.edges.iter().enumerate() {
        let relation = edge_string(edge.record, "relation");
        if matches!(
            relation.as_str(),
            "imports" | "imports_from" | "contains" | "method"
        ) || is_concept_node(graph.nodes[edge.left])
            || is_concept_node(graph.nodes[edge.right])
            || graph.is_file_node(edge.left)
            || graph.is_file_node(edge.right)
        {
            continue;
        }
        let left_source = attribute(graph.nodes[edge.left], "source_file").unwrap_or_default();
        let right_source = attribute(graph.nodes[edge.right], "source_file").unwrap_or_default();
        if left_source.is_empty() || right_source.is_empty() || left_source == right_source {
            continue;
        }
        let (score, reasons) =
            surprise_score(graph, edge, &node_community, left_source, right_source);
        let (source, target) = oriented_endpoints(graph, edge);
        candidates.push((
            score,
            position,
            SurpriseConnection {
                source: graph.nodes[source].label().to_owned(),
                target: graph.nodes[target].label().to_owned(),
                source_files: [
                    attribute(graph.nodes[source], "source_file")
                        .unwrap_or_default()
                        .to_owned(),
                    attribute(graph.nodes[target], "source_file")
                        .unwrap_or_default()
                        .to_owned(),
                ],
                confidence: defaulted_edge(edge.record, "confidence", "EXTRACTED"),
                relation,
                why: Some(if reasons.is_empty() {
                    "cross-file semantic connection".to_owned()
                } else {
                    reasons.join("; ")
                }),
                note: None,
            },
        ));
    }
    candidates.sort_by_key(|(score, position, _)| (std::cmp::Reverse(*score), *position));
    candidates
        .into_iter()
        .take(top_n)
        .map(|(_, _, item)| item)
        .collect()
}

fn cross_community_surprises(
    graph: &AnalysisGraph<'_>,
    communities: &Communities,
    top_n: usize,
) -> Vec<SurpriseConnection> {
    if communities.is_empty() {
        if graph.edges.is_empty() || graph.len() > 5000 {
            return Vec::new();
        }
        let scores = edge_betweenness(graph);
        let mut ranked = graph.edges.iter().enumerate().collect::<Vec<_>>();
        ranked.sort_by(|(left_position, _), (right_position, _)| {
            scores[*right_position]
                .partial_cmp(&scores[*left_position])
                .unwrap_or(std::cmp::Ordering::Equal)
                .then_with(|| left_position.cmp(right_position))
        });
        return ranked
            .into_iter()
            .take(top_n)
            .map(|(position, edge)| SurpriseConnection {
                source: graph.nodes[edge.left].label().to_owned(),
                target: graph.nodes[edge.right].label().to_owned(),
                source_files: [
                    attribute(graph.nodes[edge.left], "source_file")
                        .unwrap_or_default()
                        .to_owned(),
                    attribute(graph.nodes[edge.right], "source_file")
                        .unwrap_or_default()
                        .to_owned(),
                ],
                confidence: defaulted_edge(edge.record, "confidence", "EXTRACTED"),
                relation: edge_string(edge.record, "relation"),
                why: None,
                note: Some(format!(
                    "Bridges graph structure (betweenness={:.3})",
                    scores[position]
                )),
            })
            .collect();
    }
    let node_community = invert_communities(communities);
    let mut candidates = Vec::<(usize, (usize, usize), SurpriseConnection)>::new();
    for edge in &graph.edges {
        let left_community = node_community.get(&graph.nodes[edge.left].id).copied();
        let right_community = node_community.get(&graph.nodes[edge.right].id).copied();
        if left_community.is_none()
            || right_community.is_none()
            || left_community == right_community
            || graph.is_file_node(edge.left)
            || graph.is_file_node(edge.right)
        {
            continue;
        }
        let relation = edge_string(edge.record, "relation");
        if matches!(
            relation.as_str(),
            "imports" | "imports_from" | "contains" | "method"
        ) {
            continue;
        }
        let (source, target) = oriented_endpoints(graph, edge);
        let confidence = defaulted_edge(edge.record, "confidence", "EXTRACTED");
        let order = match confidence.as_str() {
            "AMBIGUOUS" => 0,
            "INFERRED" => 1,
            "EXTRACTED" => 2,
            _ => 3,
        };
        let left_community = left_community.unwrap_or_default();
        let right_community = right_community.unwrap_or_default();
        let pair = if left_community <= right_community {
            (left_community, right_community)
        } else {
            (right_community, left_community)
        };
        candidates.push((
            order,
            pair,
            SurpriseConnection {
                source: graph.nodes[source].label().to_owned(),
                target: graph.nodes[target].label().to_owned(),
                source_files: [
                    attribute(graph.nodes[source], "source_file")
                        .unwrap_or_default()
                        .to_owned(),
                    attribute(graph.nodes[target], "source_file")
                        .unwrap_or_default()
                        .to_owned(),
                ],
                confidence,
                relation,
                why: None,
                note: Some(format!(
                    "Bridges community {} → community {}",
                    left_community, right_community
                )),
            },
        ));
    }
    candidates.sort_by_key(|(order, _, _)| *order);
    let mut seen = HashSet::new();
    candidates
        .into_iter()
        .filter(|(_, pair, _)| seen.insert(*pair))
        .take(top_n)
        .map(|(_, _, item)| item)
        .collect()
}

fn surprise_score(
    graph: &AnalysisGraph<'_>,
    edge: &AnalysisEdge<'_>,
    communities: &HashMap<String, usize>,
    left_source: &str,
    right_source: &str,
) -> (i32, Vec<String>) {
    let confidence = defaulted_edge(edge.record, "confidence", "EXTRACTED");
    let relation = edge_string(edge.record, "relation");
    let left_category = file_category(left_source);
    let right_category = file_category(right_source);
    let suppressed = confidence == "INFERRED"
        && matches!(relation.as_str(), "calls" | "uses")
        && (cross_language(left_source, right_source)
            || ((left_category == "code" && right_category == "doc")
                || (left_category == "doc" && right_category == "code")));
    let mut score = if suppressed {
        0
    } else {
        match confidence.as_str() {
            "AMBIGUOUS" => 3,
            "INFERRED" => 2,
            _ => 1,
        }
    };
    let mut reasons = Vec::new();
    if matches!(confidence.as_str(), "AMBIGUOUS" | "INFERRED") {
        reasons.push(format!(
            "{} connection - not explicitly stated in source",
            confidence.to_lowercase()
        ));
    }
    if left_category != right_category && !suppressed {
        score += 2;
        reasons.push(format!(
            "crosses file types ({left_category} ↔ {right_category})"
        ));
    }
    if top_level(left_source) != top_level(right_source) && !suppressed {
        score += 2;
        reasons.push("connects across different repos/directories".to_owned());
    }
    let left_community = communities.get(&graph.nodes[edge.left].id);
    let right_community = communities.get(&graph.nodes[edge.right].id);
    if left_community.is_some()
        && right_community.is_some()
        && left_community != right_community
        && !suppressed
    {
        score += 1;
        reasons.push("bridges separate communities".to_owned());
    }
    if relation == "semantically_similar_to" {
        score = (score as f64 * 1.5) as i32;
        reasons.push("semantically similar concepts with no structural link".to_owned());
    }
    let left_degree = graph.degree(edge.left);
    let right_degree = graph.degree(edge.right);
    if left_degree.min(right_degree) <= 2 && left_degree.max(right_degree) >= 5 {
        score += 1;
        let (peripheral, hub) = if left_degree <= 2 {
            (edge.left, edge.right)
        } else {
            (edge.right, edge.left)
        };
        reasons.push(format!(
            "peripheral node `{}` unexpectedly reaches hub `{}`",
            graph.nodes[peripheral].label(),
            graph.nodes[hub].label()
        ));
    }
    (score, reasons)
}

fn node_betweenness(graph: &AnalysisGraph<'_>, sampled: bool) -> Vec<f64> {
    let sources = if sampled {
        PythonRandom::seeded(42).sample_indices(graph.len(), 100.min(graph.len()))
    } else {
        (0..graph.len()).collect()
    };
    let mut scores = vec![0.0; graph.len()];
    for source in &sources {
        let (mut stack, predecessors, paths) = shortest_paths(graph, *source);
        let mut dependency = vec![0.0; graph.len()];
        while let Some(node) = stack.pop() {
            let coefficient = (1.0 + dependency[node]) / paths[node];
            for predecessor in &predecessors[node] {
                dependency[*predecessor] += paths[*predecessor] * coefficient;
            }
            if node != *source {
                scores[node] += dependency[node];
            }
        }
    }
    let n = graph.len();
    if n > 2 {
        if sampled {
            let sampled_set = sources.iter().copied().collect::<HashSet<_>>();
            let source_scale = if sources.len() > 1 {
                1.0 / ((sources.len() - 1) * (n - 2)) as f64
            } else {
                f64::NAN
            };
            let other_scale = 1.0 / (sources.len() * (n - 2)) as f64;
            for (node, score) in scores.iter_mut().enumerate() {
                *score *= if sampled_set.contains(&node) {
                    source_scale
                } else {
                    other_scale
                };
            }
        } else {
            let scale = 1.0 / ((n - 1) * (n - 2)) as f64;
            for score in &mut scores {
                *score *= scale;
            }
        }
    }
    scores
}

fn edge_betweenness(graph: &AnalysisGraph<'_>) -> Vec<f64> {
    let mut scores = vec![0.0; graph.edges.len()];
    for source in 0..graph.len() {
        let (mut stack, predecessors, paths) = shortest_paths(graph, source);
        let mut dependency = vec![0.0; graph.len()];
        while let Some(node) = stack.pop() {
            let coefficient = (1.0 + dependency[node]) / paths[node];
            for predecessor in &predecessors[node] {
                let contribution = paths[*predecessor] * coefficient;
                if let Some(edge) = graph.edge_between(*predecessor, node) {
                    scores[edge] += contribution;
                }
                dependency[*predecessor] += contribution;
            }
        }
    }
    if graph.len() > 1 {
        let scale = 1.0 / (graph.len() * (graph.len() - 1)) as f64;
        for score in &mut scores {
            *score *= scale;
        }
    }
    scores
}

fn shortest_paths(
    graph: &AnalysisGraph<'_>,
    source: usize,
) -> (Vec<usize>, Vec<Vec<usize>>, Vec<f64>) {
    let mut stack = Vec::new();
    let mut predecessors = vec![Vec::new(); graph.len()];
    let mut paths = vec![0.0; graph.len()];
    let mut distance = vec![None; graph.len()];
    paths[source] = 1.0;
    distance[source] = Some(0);
    let mut queue = VecDeque::from([source]);
    while let Some(node) = queue.pop_front() {
        stack.push(node);
        let next_distance = distance[node].unwrap_or_default() + 1;
        for neighbor in &graph.adjacency[node] {
            if distance[*neighbor].is_none() {
                queue.push_back(*neighbor);
                distance[*neighbor] = Some(next_distance);
            }
            if distance[*neighbor] == Some(next_distance) {
                paths[*neighbor] += paths[node];
                predecessors[*neighbor].push(node);
            }
        }
    }
    (stack, predecessors, paths)
}

#[allow(clippy::too_many_arguments)]
fn enumerate_cycles(
    start: usize,
    current: usize,
    adjacency: &[Vec<usize>],
    maximum: usize,
    path: &mut Vec<usize>,
    visited: &mut HashSet<usize>,
    cycles: &mut Vec<Vec<usize>>,
    limit: usize,
) {
    if cycles.len() >= limit {
        return;
    }
    for next in &adjacency[current] {
        if *next == start {
            cycles.push(path.clone());
            if cycles.len() >= limit {
                return;
            }
        } else if path.len() < maximum && visited.insert(*next) {
            path.push(*next);
            enumerate_cycles(
                start, *next, adjacency, maximum, path, visited, cycles, limit,
            );
            path.pop();
            visited.remove(next);
        }
    }
}

struct AnalysisEdge<'a> {
    left: usize,
    right: usize,
    record: &'a EdgeRecord,
}
struct AnalysisGraph<'a> {
    nodes: Vec<&'a NodeRecord>,
    positions: HashMap<&'a str, usize>,
    edges: Vec<AnalysisEdge<'a>>,
    adjacency: Vec<Vec<usize>>,
    directed: bool,
}

impl<'a> AnalysisGraph<'a> {
    fn new(document: &'a GraphDocument) -> Self {
        let nodes = document.nodes.iter().collect::<Vec<_>>();
        let positions = nodes
            .iter()
            .enumerate()
            .map(|(index, node)| (node.id.as_str(), index))
            .collect::<HashMap<_, _>>();
        let mut edges = Vec::<AnalysisEdge<'a>>::new();
        let mut edge_positions = HashMap::<(usize, usize), usize>::new();
        let mut adjacency = vec![Vec::new(); nodes.len()];
        for record in &document.links {
            let (Some(left), Some(right)) = (
                positions.get(record.source.as_str()),
                positions.get(record.target.as_str()),
            ) else {
                continue;
            };
            let key = if document.directed || left <= right {
                (*left, *right)
            } else {
                (*right, *left)
            };
            if let Some(position) = edge_positions.get(&key) {
                edges[*position].record = record;
                continue;
            }
            edge_positions.insert(key, edges.len());
            edges.push(AnalysisEdge {
                left: *left,
                right: *right,
                record,
            });
            adjacency[*left].push(*right);
            if !document.directed && left != right {
                adjacency[*right].push(*left);
            }
        }
        Self {
            nodes,
            positions,
            edges,
            adjacency,
            directed: document.directed,
        }
    }
    fn len(&self) -> usize {
        self.nodes.len()
    }
    fn degree(&self, node: usize) -> usize {
        if self.directed {
            self.edges
                .iter()
                .map(|edge| usize::from(edge.left == node) + usize::from(edge.right == node))
                .sum()
        } else {
            self.adjacency[node]
                .iter()
                .map(|neighbor| if *neighbor == node { 2 } else { 1 })
                .sum()
        }
    }
    fn is_file_node(&self, node: usize) -> bool {
        let record = self.nodes[node];
        let label = record
            .attributes
            .get("label")
            .and_then(Value::as_str)
            .unwrap_or_default();
        if label.is_empty() {
            return false;
        }
        let source = attribute(record, "source_file").unwrap_or_default();
        if !source.is_empty()
            && Path::new(&source)
                .file_name()
                .and_then(|name| name.to_str())
                == Some(label)
        {
            return true;
        }
        (label.starts_with('.') && label.ends_with("()"))
            || (label.ends_with("()") && self.degree(node) <= 1)
    }
    fn incident_edges(&self, node: usize) -> Vec<&AnalysisEdge<'a>> {
        self.edges
            .iter()
            .filter(|edge| edge.left == node || edge.right == node)
            .collect()
    }
    fn edge_between(&self, left: usize, right: usize) -> Option<usize> {
        self.edges.iter().position(|edge| {
            (edge.left == left && edge.right == right)
                || (!self.directed && edge.left == right && edge.right == left)
        })
    }
}

fn oriented_endpoints(graph: &AnalysisGraph<'_>, edge: &AnalysisEdge<'_>) -> (usize, usize) {
    let source = edge
        .record
        .attributes
        .get("_src")
        .and_then(Value::as_str)
        .and_then(|id| graph.positions.get(id))
        .copied()
        .unwrap_or(edge.left);
    let target = edge
        .record
        .attributes
        .get("_tgt")
        .and_then(Value::as_str)
        .and_then(|id| graph.positions.get(id))
        .copied()
        .unwrap_or(edge.right);
    (source, target)
}
fn oriented_other(graph: &AnalysisGraph<'_>, edge: &AnalysisEdge<'_>, node: usize) -> usize {
    let (source, target) = oriented_endpoints(graph, edge);
    if source == node { target } else { source }
}
fn invert_communities(communities: &Communities) -> HashMap<String, usize> {
    communities
        .iter()
        .flat_map(|(community, nodes)| nodes.iter().map(move |node| (node.clone(), *community)))
        .collect()
}
fn is_concept_node(node: &NodeRecord) -> bool {
    let source = attribute(node, "source_file").unwrap_or_default();
    source.is_empty() || !source.rsplit('/').next().unwrap_or_default().contains('.')
}
fn is_json_key_node(node: &NodeRecord) -> bool {
    attribute(node, "source_file").is_some_and(|source| source.to_lowercase().ends_with(".json"))
        && JSON_NOISE_LABELS.contains(&node.label().trim().to_lowercase().as_str())
}
fn attribute<'a>(node: &'a NodeRecord, key: &str) -> Option<&'a str> {
    node.attributes.get(key).and_then(Value::as_str)
}
fn edge_string(edge: &EdgeRecord, key: &str) -> String {
    edge.attributes
        .get(key)
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_owned()
}
fn defaulted_edge(edge: &EdgeRecord, key: &str, default: &str) -> String {
    edge.attributes
        .get(key)
        .and_then(Value::as_str)
        .unwrap_or(default)
        .to_owned()
}
fn diff_edge_key(directed: bool, edge: &EdgeRecord) -> (String, String, String) {
    let (source, target) = if directed || edge.source <= edge.target {
        (edge.source.clone(), edge.target.clone())
    } else {
        (edge.target.clone(), edge.source.clone())
    };
    (source, target, edge_string(edge, "relation"))
}
fn diff_edge(edge: &EdgeRecord) -> DiffEdge {
    DiffEdge {
        source: edge.source.clone(),
        target: edge.target.clone(),
        relation: edge_string(edge, "relation"),
        confidence: edge_string(edge, "confidence"),
    }
}
fn plural(count: usize, singular: &str, plural: &str) -> String {
    format!("{count} {}", if count == 1 { singular } else { plural })
}
fn file_index(
    file: &str,
    files: &mut Vec<String>,
    positions: &mut HashMap<String, usize>,
) -> usize {
    if let Some(position) = positions.get(file) {
        *position
    } else {
        let position = files.len();
        files.push(file.to_owned());
        positions.insert(file.to_owned(), position);
        position
    }
}
fn top_level(path: &str) -> &str {
    path.split('/').next().unwrap_or(path)
}
fn extension(path: &str) -> String {
    Path::new(path)
        .extension()
        .and_then(|value| value.to_str())
        .map(|value| format!(".{}", value.to_lowercase()))
        .unwrap_or_default()
}
fn cross_language(left: &str, right: &str) -> bool {
    let left = language_family(&extension(left));
    let right = language_family(&extension(right));
    left.is_some() && right.is_some() && left != right
}
fn language_family(extension: &str) -> Option<&'static str> {
    match extension {
        ".py" | ".pyw" => Some("python"),
        ".js" | ".jsx" | ".mjs" | ".cjs" | ".ejs" | ".ts" | ".tsx" | ".mts" | ".cts" | ".vue"
        | ".svelte" => Some("js"),
        ".go" => Some("go"),
        ".rs" => Some("rust"),
        ".java" | ".kt" | ".kts" | ".scala" => Some("jvm"),
        ".c" | ".h" | ".cpp" | ".cc" | ".cxx" | ".hpp" => Some("c"),
        ".rb" | ".rake" => Some("ruby"),
        ".swift" => Some("swift"),
        ".cs" => Some("dotnet"),
        ".php" => Some("php"),
        ".r" => Some("r"),
        _ => None,
    }
}
fn file_category(path: &str) -> &'static str {
    let ext = extension(path);
    if matches!(
        ext.as_str(),
        ".py"
            | ".ts"
            | ".tsx"
            | ".mts"
            | ".cts"
            | ".js"
            | ".jsx"
            | ".mjs"
            | ".cjs"
            | ".ejs"
            | ".ets"
            | ".go"
            | ".rs"
            | ".java"
            | ".groovy"
            | ".gradle"
            | ".cpp"
            | ".cc"
            | ".cxx"
            | ".c"
            | ".h"
            | ".hpp"
            | ".cu"
            | ".cuh"
            | ".metal"
            | ".rb"
            | ".rake"
            | ".swift"
            | ".kt"
            | ".kts"
            | ".cs"
            | ".scala"
            | ".php"
            | ".lua"
            | ".luau"
            | ".toc"
            | ".zig"
            | ".ps1"
            | ".psm1"
            | ".psd1"
            | ".ex"
            | ".exs"
            | ".m"
            | ".mm"
            | ".jl"
            | ".vue"
            | ".svelte"
            | ".astro"
            | ".dart"
            | ".v"
            | ".sv"
            | ".svh"
            | ".sql"
            | ".r"
            | ".f"
            | ".f90"
            | ".f95"
            | ".f03"
            | ".f08"
            | ".pas"
            | ".pp"
            | ".dpr"
            | ".dpk"
            | ".lpr"
            | ".inc"
            | ".dfm"
            | ".lfm"
            | ".lpk"
            | ".sh"
            | ".bash"
            | ".json"
            | ".tf"
            | ".tfvars"
            | ".hcl"
            | ".dm"
            | ".dme"
            | ".dmi"
            | ".dmm"
            | ".dmf"
            | ".sln"
            | ".slnx"
            | ".csproj"
            | ".fsproj"
            | ".vbproj"
            | ".xaml"
            | ".razor"
            | ".cshtml"
            | ".cls"
            | ".trigger"
    ) {
        "code"
    } else if ext == ".pdf" {
        "paper"
    } else if matches!(
        ext.as_str(),
        ".png" | ".jpg" | ".jpeg" | ".gif" | ".webp" | ".svg"
    ) {
        "image"
    } else {
        "doc"
    }
}

trait BooleanValue {
    fn is_boolean_and_true(&self) -> bool;
}
impl BooleanValue for Value {
    fn is_boolean_and_true(&self) -> bool {
        self.as_bool() == Some(true)
    }
}
