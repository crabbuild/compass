use std::collections::{BTreeMap, BTreeSet, VecDeque};

use ahash::{AHashMap as HashMap, AHashSet as HashSet};
use std::path::Path;

use compass_model::{EdgeRecord, GraphDocument, NodeRecord};
use rayon::prelude::*;
use serde::Serialize;
use sha2::{Digest, Sha256};

use crate::cluster::{Communities, PythonRandom};

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

const COMMUNITY_GAP_MIN_REAL_NODES: usize = 4;
const COMMUNITY_GAP_MIN_CONNECTANCE: f64 = 1.2;
const COMMUNITY_GAP_MAX_NEIGHBOR_COMMUNITIES: usize = 32;
const COMMUNITY_GAP_MAX_PAIRS: usize = 200_000;
const COMMUNITY_GAP_MAX_QUESTIONS: usize = 3;
const ANALYSIS_LABEL_MAX_CHARS: usize = 160;

// These relations describe graph wiring rather than a topical or semantic
// connection. They may make two communities adjacent without making them a
// useful structural-gap candidate.
const COMMUNITY_GAP_WIRING_RELATIONS: &[&str] = &[
    "contains",
    "declares",
    "defines",
    "imports",
    "imports_from",
    "member_of",
    "method",
    "re_exports",
];

#[derive(Clone, Debug, PartialEq, Eq, Serialize, serde::Deserialize)]
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

/// Versioned, bounded evidence for topology that deserves investigation.
///
/// These are observations about the published graph. They are deliberately
/// separate from graph edges and from the prose questions derived from them.
pub const GRAPH_INSIGHTS_SCHEMA: &str = "compass.graph-insights/1";

const BLIND_SPOT_MAX_COMPONENTS: usize = 32;
const BLIND_SPOT_MAX_COMPONENT_MEMBERS: usize = 64;
const BLIND_SPOT_MAX_WITNESSES: usize = 8;

#[derive(Clone, Debug, PartialEq, Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BlindSpotReport {
    pub schema: String,
    pub community_gaps: Vec<CommunityGap>,
    pub disconnected_components: Vec<DisconnectedComponent>,
    pub disconnected_component_count: usize,
    pub largest_component_size: usize,
    pub omissions: BlindSpotOmissions,
    pub limits: BlindSpotLimits,
}

#[derive(Clone, Debug, PartialEq, Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CommunityGap {
    pub id: String,
    pub left_community: usize,
    pub right_community: usize,
    pub left_anchor: String,
    pub right_anchor: String,
    pub left_label: String,
    pub right_label: String,
    pub score: f64,
    pub shared_intermediary_count: usize,
    pub shared_intermediaries: Vec<BlindSpotNode>,
    pub direct_topical_edge_count: usize,
    pub direct_topical_edges: Vec<BlindSpotEdge>,
    pub omitted_shared_intermediaries: usize,
    pub omitted_direct_topical_edges: usize,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DisconnectedComponent {
    pub id: String,
    pub real_node_count: usize,
    pub members: Vec<BlindSpotNode>,
    pub omitted_members: usize,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BlindSpotNode {
    pub id: String,
    pub label: String,
    pub source_file: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BlindSpotEdge {
    pub source: String,
    pub target: String,
    pub relation: String,
    pub confidence: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BlindSpotOmissions {
    pub candidate_pair_limit_reached: bool,
    pub community_gaps: usize,
    pub disconnected_components: usize,
    pub component_members: usize,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BlindSpotLimits {
    pub max_candidate_pairs: usize,
    pub max_community_gaps: usize,
    pub max_shared_intermediaries: usize,
    pub max_direct_topical_edges: usize,
    pub max_disconnected_components: usize,
    pub max_component_members: usize,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GraphInsights {
    pub gods: Vec<GodNode>,
    pub surprises: Vec<SurpriseConnection>,
    pub questions: Vec<SuggestedQuestion>,
    pub blind_spots: BlindSpotReport,
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
    god_nodes_in(&graph, top_n)
}

fn god_nodes_in(graph: &AnalysisGraph<'_>, top_n: usize) -> Vec<GodNode> {
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
    surprising_connections_in(&graph, communities, top_n)
}

fn surprising_connections_in(
    graph: &AnalysisGraph<'_>,
    communities: &Communities,
    top_n: usize,
) -> Vec<SurpriseConnection> {
    let source_count = graph
        .nodes
        .iter()
        .filter_map(|node| attribute(node, "source_file"))
        .filter(|source| !source.is_empty())
        .collect::<HashSet<_>>()
        .len();
    if source_count > 1 {
        let cross_file = cross_file_surprises(graph, communities, top_n);
        if !cross_file.is_empty() {
            return cross_file;
        }
    }
    cross_community_surprises(graph, communities, top_n)
}

#[must_use]
pub fn suggest_questions(
    document: &GraphDocument,
    communities: &Communities,
    community_labels: &BTreeMap<usize, String>,
    top_n: usize,
) -> Vec<SuggestedQuestion> {
    let graph = AnalysisGraph::new(document);
    suggest_questions_in(&graph, communities, community_labels, top_n).0
}

fn suggest_questions_in(
    graph: &AnalysisGraph<'_>,
    communities: &Communities,
    community_labels: &BTreeMap<usize, String>,
    top_n: usize,
) -> (Vec<SuggestedQuestion>, BlindSpotReport) {
    let node_community = invert_communities(communities);
    let cohesion = community_cohesion_scores(graph, communities, &node_community);
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
        let centrality = node_betweenness(graph, graph.len() > 1000);
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
                let other = oriented_other(graph, edge, node);
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
                && graph.nodes[*node].string("file_type") != "rationale"
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
        let score = cohesion.get(community).copied().unwrap_or(1.0);
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
    let blind_spots = blind_spots_in(graph, communities, community_labels);
    questions.extend(blind_spot_questions(&blind_spots));
    if questions.is_empty() {
        questions.push(SuggestedQuestion {
            kind: "no_signal".to_owned(),
            question: None,
            why: "Not enough signal to generate questions. This usually means the corpus has no AMBIGUOUS edges, no bridge nodes, no INFERRED relationships, and all communities are tightly cohesive. Add more files or run with --mode deep to extract richer edges.".to_owned(),
        });
    }
    (prioritize_questions(questions, top_n), blind_spots)
}

fn blind_spot_questions(report: &BlindSpotReport) -> Vec<SuggestedQuestion> {
    let mut questions = report
        .community_gaps
        .iter()
        .map(|gap| SuggestedQuestion {
            kind: "community_gap".to_owned(),
            question: Some(format!(
                "What evidence would directly connect `{}` and `{}` (for example, through a shared intermediary)?",
                gap.left_label, gap.right_label
            )),
            why: format!(
                "Structural gap score {:.4}: {} shared two-hop intermediaries and {} direct topical edges; wiring-only relations are excluded.",
                gap.score, gap.shared_intermediary_count, gap.direct_topical_edge_count
            ),
        })
        .collect::<Vec<_>>();
    if report.omissions.candidate_pair_limit_reached || report.omissions.community_gaps > 0 {
        questions.push(SuggestedQuestion {
            kind: "community_gap_limit".to_owned(),
            question: None,
            why: format!(
                "Structural-gap analysis is bounded at {} candidate pairs and {} displayed gaps; some eligible evidence was omitted.",
                report.limits.max_candidate_pairs, report.limits.max_community_gaps
            ),
        });
    }
    if report.disconnected_component_count > 1 {
        questions.push(SuggestedQuestion {
            kind: "disconnected_components".to_owned(),
            question: Some(format!(
                "Which relationships are missing between the {} disconnected source-backed components?",
                report.disconnected_component_count
            )),
            why: format!(
                "The graph contains {} weakly connected source-backed components; the largest has {} real nodes. File, concept, and JSON-key-only components are not counted.",
                report.disconnected_component_count, report.largest_component_size
            ),
        });
    }
    questions
}

fn prioritize_questions(questions: Vec<SuggestedQuestion>, top_n: usize) -> Vec<SuggestedQuestion> {
    if top_n == 0 {
        return Vec::new();
    }
    let mut diagnostics: [Vec<SuggestedQuestion>; 3] = [Vec::new(), Vec::new(), Vec::new()];
    let mut ordinary = Vec::new();
    for question in questions {
        let priority = match question.kind.as_str() {
            "community_gap" => Some(0),
            "disconnected_components" => Some(1),
            "community_gap_limit" => Some(2),
            _ => None,
        };
        if let Some(priority) = priority {
            diagnostics[priority].push(question);
        } else {
            ordinary.push(question);
        }
    }
    // Keep the structural diagnostics category-aware: one large set of gaps
    // must not hide the fact that the graph is also disconnected (or that the
    // analysis itself was bounded). Fill one slot per category first, then
    // use the normal priority order for the remaining slots.
    let mut selected = Vec::new();
    for bucket in &mut diagnostics {
        if selected.len() >= top_n {
            break;
        }
        if !bucket.is_empty() {
            selected.push(bucket.remove(0));
        }
    }
    for bucket in diagnostics {
        selected.extend(bucket);
    }
    selected.extend(ordinary);
    selected.truncate(top_n);
    selected
}

struct CommunityPairScore {
    proximity: f64,
    intermediaries: Vec<usize>,
    omitted_intermediaries: usize,
    direct_edges: usize,
    direct_witnesses: Vec<BlindSpotEdge>,
}

fn blind_spots_in(
    graph: &AnalysisGraph<'_>,
    communities: &Communities,
    community_labels: &BTreeMap<usize, String>,
) -> BlindSpotReport {
    let node_community = invert_communities(communities);
    let relation_edges = sorted_relation_edges(graph);

    let mut real_members = BTreeMap::<usize, Vec<usize>>::new();
    for (community, members) in communities {
        let mut positions = members
            .iter()
            .filter_map(|member| graph.positions.get(member.as_str()).copied())
            .filter(|position| is_community_gap_real_node(graph, *position))
            .collect::<Vec<_>>();
        positions.sort_unstable();
        positions.dedup();
        real_members.insert(*community, positions);
    }

    let mut internal_edges = BTreeMap::<usize, BTreeSet<(usize, usize)>>::new();
    for edge in &relation_edges {
        let left = node_community.get(&graph.nodes[edge.left].id);
        let right = node_community.get(&graph.nodes[edge.right].id);
        if let (Some(left), Some(right)) = (left, right)
            && left == right
            && !is_community_gap_wiring_relation(edge.record)
            && is_community_gap_real_node(graph, edge.left)
            && is_community_gap_real_node(graph, edge.right)
        {
            let endpoints = if edge.left <= edge.right {
                (edge.left, edge.right)
            } else {
                (edge.right, edge.left)
            };
            internal_edges.entry(*left).or_default().insert(endpoints);
        }
    }

    let eligible = real_members
        .into_iter()
        .filter(|(community, members)| {
            let internal = internal_edges.get(community).map_or(0, BTreeSet::len);
            let connectance = if members.is_empty() {
                0.0
            } else {
                (internal as f64 * 2.0) / members.len() as f64
            };
            members.len() >= COMMUNITY_GAP_MIN_REAL_NODES
                && connectance >= COMMUNITY_GAP_MIN_CONNECTANCE
        })
        .collect::<BTreeMap<_, _>>();

    let mut topical_adjacency = vec![Vec::<usize>::new(); graph.len()];
    for edge in &relation_edges {
        if is_community_gap_wiring_relation(edge.record) {
            continue;
        }
        topical_adjacency[edge.left].push(edge.right);
        topical_adjacency[edge.right].push(edge.left);
    }
    for neighbors in &mut topical_adjacency {
        neighbors.sort_by(|left, right| graph.nodes[*left].id.cmp(&graph.nodes[*right].id));
        neighbors.dedup();
    }

    let mut pair_scores = BTreeMap::<(usize, usize), CommunityPairScore>::new();
    let mut pair_budget_exhausted = false;
    let mut middles = (0..graph.len()).collect::<Vec<_>>();
    middles.sort_by(|left, right| graph.nodes[*left].id.cmp(&graph.nodes[*right].id));
    for middle in middles {
        if !is_community_gap_real_node(graph, middle) {
            continue;
        }
        let neighbors = &topical_adjacency[middle];
        let mut neighbor_communities = BTreeSet::new();
        for neighbor in neighbors {
            if is_community_gap_real_node(graph, *neighbor)
                && let Some(community) = node_community.get(&graph.nodes[*neighbor].id)
                && eligible.contains_key(community)
            {
                neighbor_communities.insert(*community);
            }
        }
        if neighbor_communities.len() < 2
            || neighbor_communities.len() > COMMUNITY_GAP_MAX_NEIGHBOR_COMMUNITIES
        {
            continue;
        }
        let neighbor_communities = neighbor_communities.into_iter().collect::<Vec<_>>();
        let weight = 1.0 / (graph.degree(middle).max(1) as f64).sqrt();
        for left_index in 0..neighbor_communities.len().saturating_sub(1) {
            for right_index in (left_index + 1)..neighbor_communities.len() {
                let pair = (
                    neighbor_communities[left_index],
                    neighbor_communities[right_index],
                );
                if !pair_scores.contains_key(&pair) && pair_scores.len() >= COMMUNITY_GAP_MAX_PAIRS
                {
                    pair_budget_exhausted = true;
                    continue;
                }
                let score = pair_scores.entry(pair).or_insert(CommunityPairScore {
                    proximity: 0.0,
                    intermediaries: Vec::new(),
                    omitted_intermediaries: 0,
                    direct_edges: 0,
                    direct_witnesses: Vec::new(),
                });
                score.proximity += weight;
                if score.intermediaries.len() < BLIND_SPOT_MAX_WITNESSES {
                    score.intermediaries.push(middle);
                } else {
                    score.omitted_intermediaries = score.omitted_intermediaries.saturating_add(1);
                }
            }
        }
    }

    for edge in &relation_edges {
        if is_community_gap_wiring_relation(edge.record) {
            continue;
        }
        if !is_community_gap_real_node(graph, edge.left)
            || !is_community_gap_real_node(graph, edge.right)
        {
            continue;
        }
        let Some(left) = node_community.get(&graph.nodes[edge.left].id) else {
            continue;
        };
        let Some(right) = node_community.get(&graph.nodes[edge.right].id) else {
            continue;
        };
        if left == right || !eligible.contains_key(left) || !eligible.contains_key(right) {
            continue;
        }
        let pair = if left < right {
            (*left, *right)
        } else {
            (*right, *left)
        };
        if let Some(score) = pair_scores.get_mut(&pair) {
            score.direct_edges = score.direct_edges.saturating_add(1);
            let witness = blind_spot_edge(graph, edge.record);
            if !score
                .direct_witnesses
                .iter()
                .any(|candidate| candidate == &witness)
                && score.direct_witnesses.len() < BLIND_SPOT_MAX_WITNESSES
            {
                score.direct_witnesses.push(witness);
            }
        }
    }

    let mut ranked = pair_scores
        .into_iter()
        .map(|((left, right), evidence)| {
            let score = evidence.proximity / (1.0 + evidence.direct_edges as f64);
            (score, left, right, evidence)
        })
        .filter(|(score, _, _, _)| *score > 0.0)
        .collect::<Vec<_>>();
    ranked.sort_by(|left, right| {
        right
            .0
            .partial_cmp(&left.0)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| left.1.cmp(&right.1))
            .then_with(|| left.2.cmp(&right.2))
    });

    let total_ranked = ranked.len();
    let community_gaps = ranked
        .into_iter()
        .take(COMMUNITY_GAP_MAX_QUESTIONS)
        .filter_map(|(score, left, right, evidence)| {
            let left_members = eligible.get(&left)?;
            let right_members = eligible.get(&right)?;
            let left_anchor = community_gap_anchor(graph, left_members);
            let right_anchor = community_gap_anchor(graph, right_members);
            let left_label = community_gap_label(left, community_labels, graph, left_members);
            let right_label = community_gap_label(right, community_labels, graph, right_members);
            let shared_intermediaries = evidence
                .intermediaries
                .iter()
                .map(|position| blind_spot_node(graph.nodes[*position]))
                .collect::<Vec<_>>();
            let direct_witness_count = evidence.direct_witnesses.len();
            Some(CommunityGap {
                id: community_gap_id(&left_anchor, &right_anchor),
                left_community: left,
                right_community: right,
                left_anchor,
                right_anchor,
                left_label,
                right_label,
                score,
                shared_intermediary_count: evidence
                    .intermediaries
                    .len()
                    .saturating_add(evidence.omitted_intermediaries),
                shared_intermediaries,
                direct_topical_edge_count: evidence.direct_edges,
                direct_topical_edges: evidence.direct_witnesses,
                omitted_shared_intermediaries: evidence.omitted_intermediaries,
                omitted_direct_topical_edges: evidence
                    .direct_edges
                    .saturating_sub(direct_witness_count),
            })
        })
        .collect::<Vec<_>>();
    let (
        disconnected_components,
        disconnected_component_count,
        largest_component_size,
        omitted_disconnected_components,
        omitted_component_members,
    ) = disconnected_components_in(graph);
    let (disconnected_components, omitted_disconnected_components, omitted_component_members) =
        if disconnected_component_count > 1 {
            (
                disconnected_components,
                omitted_disconnected_components,
                omitted_component_members,
            )
        } else {
            (Vec::new(), 0, 0)
        };
    BlindSpotReport {
        schema: GRAPH_INSIGHTS_SCHEMA.to_owned(),
        community_gaps,
        disconnected_components,
        disconnected_component_count,
        largest_component_size,
        omissions: BlindSpotOmissions {
            candidate_pair_limit_reached: pair_budget_exhausted,
            community_gaps: total_ranked.saturating_sub(COMMUNITY_GAP_MAX_QUESTIONS),
            disconnected_components: omitted_disconnected_components,
            component_members: omitted_component_members,
        },
        limits: BlindSpotLimits {
            max_candidate_pairs: COMMUNITY_GAP_MAX_PAIRS,
            max_community_gaps: COMMUNITY_GAP_MAX_QUESTIONS,
            max_shared_intermediaries: BLIND_SPOT_MAX_WITNESSES,
            max_direct_topical_edges: BLIND_SPOT_MAX_WITNESSES,
            max_disconnected_components: BLIND_SPOT_MAX_COMPONENTS,
            max_component_members: BLIND_SPOT_MAX_COMPONENT_MEMBERS,
        },
    }
}

#[must_use]
pub fn blind_spot_report(
    document: &GraphDocument,
    communities: &Communities,
    community_labels: &BTreeMap<usize, String>,
) -> BlindSpotReport {
    let graph = AnalysisGraph::new(document);
    blind_spots_in(&graph, communities, community_labels)
}

fn blind_spot_node(node: &NodeRecord) -> BlindSpotNode {
    BlindSpotNode {
        id: node.id.clone(),
        label: bounded_analysis_label(node.label()),
        source_file: attribute(node, "source_file")
            .filter(|source| !source.is_empty())
            .map(str::to_owned),
    }
}

fn community_gap_anchor(graph: &AnalysisGraph<'_>, members: &[usize]) -> String {
    members
        .iter()
        .copied()
        .min_by(|left, right| graph.nodes[*left].id.cmp(&graph.nodes[*right].id))
        .map_or_else(String::new, |position| graph.nodes[position].id.clone())
}

fn community_gap_id(left_anchor: &str, right_anchor: &str) -> String {
    let (left, right) = if left_anchor <= right_anchor {
        (left_anchor, right_anchor)
    } else {
        (right_anchor, left_anchor)
    };
    let mut input = Vec::with_capacity(left.len() + right.len() + 1);
    input.extend_from_slice(left.as_bytes());
    input.push(0);
    input.extend_from_slice(right.as_bytes());
    format!("community-gap-{:x}", Sha256::digest(input))
}

fn blind_spot_edge(graph: &AnalysisGraph<'_>, edge: &EdgeRecord) -> BlindSpotEdge {
    let (source, target) = if graph.directed || edge.source <= edge.target {
        (edge.source.clone(), edge.target.clone())
    } else {
        (edge.target.clone(), edge.source.clone())
    };
    BlindSpotEdge {
        source,
        target,
        relation: edge_string(edge, "relation"),
        confidence: edge_string(edge, "confidence"),
    }
}

fn disconnected_components_in(
    graph: &AnalysisGraph<'_>,
) -> (Vec<DisconnectedComponent>, usize, usize, usize, usize) {
    if graph.len() == 0 {
        return (Vec::new(), 0, 0, 0, 0);
    }
    let adjacency = undirected_adjacency(graph);
    let mut visited = vec![false; graph.len()];
    let mut component_count = 0_usize;
    let mut largest_component = 0_usize;
    let mut candidates = Vec::<DisconnectedComponent>::new();
    for start in 0..graph.len() {
        if visited[start] {
            continue;
        }
        visited[start] = true;
        let mut queue = VecDeque::from([start]);
        let mut real_nodes = Vec::new();
        while let Some(node) = queue.pop_front() {
            if is_community_gap_real_node(graph, node) {
                real_nodes.push(node);
            }
            for neighbor in &adjacency[node] {
                if !visited[*neighbor] {
                    visited[*neighbor] = true;
                    queue.push_back(*neighbor);
                }
            }
        }
        if real_nodes.len() < 2 {
            continue;
        }
        real_nodes.sort_by(|left, right| graph.nodes[*left].id.cmp(&graph.nodes[*right].id));
        component_count = component_count.saturating_add(1);
        largest_component = largest_component.max(real_nodes.len());
        let id = format!("component:{}", graph.nodes[real_nodes[0]].id);
        let members = real_nodes
            .iter()
            .take(BLIND_SPOT_MAX_COMPONENT_MEMBERS)
            .map(|position| blind_spot_node(graph.nodes[*position]))
            .collect::<Vec<_>>();
        let omitted = real_nodes.len().saturating_sub(members.len());
        let candidate = DisconnectedComponent {
            id: id.clone(),
            real_node_count: real_nodes.len(),
            members,
            omitted_members: omitted,
        };
        candidates.push(candidate);
    }
    candidates.sort_by(|left, right| {
        right
            .real_node_count
            .cmp(&left.real_node_count)
            .then_with(|| left.id.cmp(&right.id))
    });
    let omitted_components = candidates.len().saturating_sub(BLIND_SPOT_MAX_COMPONENTS);
    let retained_count = candidates.len().min(BLIND_SPOT_MAX_COMPONENTS);
    let omitted_members = candidates
        .iter()
        .skip(retained_count)
        .fold(0_usize, |total, component| {
            total.saturating_add(component.real_node_count)
        })
        .saturating_add(
            candidates
                .iter()
                .take(retained_count)
                .fold(0_usize, |total, component| {
                    total.saturating_add(component.omitted_members)
                }),
        );
    let components = candidates
        .into_iter()
        .take(BLIND_SPOT_MAX_COMPONENTS)
        .collect::<Vec<_>>();
    (
        components,
        component_count,
        largest_component,
        omitted_components,
        omitted_members,
    )
}

fn community_gap_label(
    community: usize,
    community_labels: &BTreeMap<usize, String>,
    graph: &AnalysisGraph<'_>,
    members: &[usize],
) -> String {
    if let Some(label) = community_labels.get(&community)
        && !label.is_empty()
    {
        return bounded_analysis_label(label);
    }
    let representative = members.iter().copied().min_by(|left, right| {
        graph
            .degree(*right)
            .cmp(&graph.degree(*left))
            .then_with(|| graph.nodes[*left].id.cmp(&graph.nodes[*right].id))
    });
    representative.map_or_else(
        || format!("Community {community}"),
        |position| bounded_analysis_label(graph.nodes[position].label()),
    )
}

fn bounded_analysis_label(value: &str) -> String {
    value.chars().take(ANALYSIS_LABEL_MAX_CHARS).collect()
}

fn is_community_gap_real_node(graph: &AnalysisGraph<'_>, position: usize) -> bool {
    !graph.is_file_node(position)
        && !is_concept_node(graph.nodes[position])
        && !is_json_key_node(graph.nodes[position])
}

fn is_community_gap_wiring_relation(edge: &EdgeRecord) -> bool {
    COMMUNITY_GAP_WIRING_RELATIONS.contains(&edge_string(edge, "relation").as_str())
}

fn undirected_adjacency(graph: &AnalysisGraph<'_>) -> Vec<Vec<usize>> {
    let mut adjacency = vec![Vec::<usize>::new(); graph.len()];
    for edge in &graph.edges {
        adjacency[edge.left].push(edge.right);
        adjacency[edge.right].push(edge.left);
    }
    for neighbors in &mut adjacency {
        neighbors.sort_unstable();
        neighbors.dedup();
    }
    adjacency
}

#[must_use]
pub fn graph_insights(
    document: &GraphDocument,
    communities: &Communities,
    community_labels: &BTreeMap<usize, String>,
    god_limit: usize,
    surprise_limit: usize,
    question_limit: usize,
) -> (
    Vec<GodNode>,
    Vec<SurpriseConnection>,
    Vec<SuggestedQuestion>,
) {
    let insights = graph_insights_with_blind_spots(
        document,
        communities,
        community_labels,
        god_limit,
        surprise_limit,
        question_limit,
    );
    (insights.gods, insights.surprises, insights.questions)
}

#[must_use]
pub fn graph_insights_with_blind_spots(
    document: &GraphDocument,
    communities: &Communities,
    community_labels: &BTreeMap<usize, String>,
    god_limit: usize,
    surprise_limit: usize,
    question_limit: usize,
) -> GraphInsights {
    let graph = AnalysisGraph::new(document);
    let (gods, (surprises, (questions, blind_spots))) = rayon::join(
        || god_nodes_in(&graph, god_limit),
        || {
            rayon::join(
                || surprising_connections_in(&graph, communities, surprise_limit),
                || suggest_questions_in(&graph, communities, community_labels, question_limit),
            )
        },
    );
    GraphInsights {
        gods,
        surprises,
        questions,
        blind_spots,
    }
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
            || edge.record.boolean("deferred") == Some(true)
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
    // Each Brandes traversal is independent. Collecting from an indexed parallel
    // iterator retains source order, and the sequential merge below retains the
    // exact floating-point accumulation order of the previous implementation.
    let source_scores = sources
        .par_iter()
        .map(|source| single_source_node_betweenness(graph, *source))
        .collect::<Vec<_>>();
    let mut scores = vec![0.0; graph.len()];
    for source_score in source_scores {
        for (score, contribution) in scores.iter_mut().zip(source_score) {
            *score += contribution;
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

fn single_source_node_betweenness(graph: &AnalysisGraph<'_>, source: usize) -> Vec<f64> {
    let (mut stack, predecessors, paths) = shortest_paths(graph, source);
    let mut dependency = vec![0.0; graph.len()];
    let mut scores = vec![0.0; graph.len()];
    while let Some(node) = stack.pop() {
        let coefficient = (1.0 + dependency[node]) / paths[node];
        for predecessor in &predecessors[node] {
            dependency[*predecessor] += paths[*predecessor] * coefficient;
        }
        if node != source {
            scores[node] = dependency[node];
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

fn sorted_relation_edges<'a>(graph: &'a AnalysisGraph<'a>) -> Vec<&'a AnalysisEdge<'a>> {
    let mut edges = graph.relation_edges.iter().collect::<Vec<_>>();
    edges.sort_by(|left, right| {
        relation_edge_sort_key(graph, left).cmp(&relation_edge_sort_key(graph, right))
    });
    edges
}

fn relation_edge_sort_key(
    graph: &AnalysisGraph<'_>,
    edge: &AnalysisEdge<'_>,
) -> (String, String, String, String) {
    let source = edge.record.source.clone();
    let target = edge.record.target.clone();
    let (source, target) = if graph.directed || source <= target {
        (source, target)
    } else {
        (target, source)
    };
    (
        source,
        target,
        edge_string(edge.record, "relation"),
        edge_string(edge.record, "confidence"),
    )
}

struct AnalysisGraph<'a> {
    nodes: Vec<&'a NodeRecord>,
    positions: HashMap<&'a str, usize>,
    edges: Vec<AnalysisEdge<'a>>,
    relation_edges: Vec<AnalysisEdge<'a>>,
    adjacency: Vec<Vec<usize>>,
    degrees: Vec<usize>,
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
        let mut relation_edges = Vec::<AnalysisEdge<'a>>::new();
        let mut edge_positions = HashMap::<(usize, usize), usize>::new();
        let mut adjacency = vec![Vec::new(); nodes.len()];
        let mut degrees = vec![0; nodes.len()];
        for record in &document.links {
            let (Some(left), Some(right)) = (
                positions.get(record.source.as_str()),
                positions.get(record.target.as_str()),
            ) else {
                continue;
            };
            relation_edges.push(AnalysisEdge {
                left: *left,
                right: *right,
                record,
            });
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
            degrees[*left] += 1;
            degrees[*right] += 1;
            adjacency[*left].push(*right);
            if !document.directed && left != right {
                adjacency[*right].push(*left);
            }
        }
        Self {
            nodes,
            positions,
            edges,
            relation_edges,
            adjacency,
            degrees,
            directed: document.directed,
        }
    }
    fn len(&self) -> usize {
        self.nodes.len()
    }
    fn degree(&self, node: usize) -> usize {
        self.degrees[node]
    }
    fn is_file_node(&self, node: usize) -> bool {
        let record = self.nodes[node];
        let label = record.label();
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
    let source = graph
        .positions
        .get(edge.record.semantic_source())
        .copied()
        .unwrap_or(edge.left);
    let target = graph
        .positions
        .get(edge.record.semantic_target())
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

fn community_cohesion_scores(
    graph: &AnalysisGraph<'_>,
    communities: &Communities,
    node_community: &HashMap<String, usize>,
) -> HashMap<usize, f64> {
    let mut internal_edges = HashMap::<usize, usize>::new();
    for edge in &graph.edges {
        let left = node_community.get(&graph.nodes[edge.left].id);
        if let Some(community) = left
            && node_community.get(&graph.nodes[edge.right].id) == Some(community)
        {
            *internal_edges.entry(*community).or_default() += 1;
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
fn is_concept_node(node: &NodeRecord) -> bool {
    let source = attribute(node, "source_file").unwrap_or_default();
    source.is_empty() || !source.rsplit('/').next().unwrap_or_default().contains('.')
}
fn is_json_key_node(node: &NodeRecord) -> bool {
    attribute(node, "source_file").is_some_and(|source| source.to_lowercase().ends_with(".json"))
        && JSON_NOISE_LABELS.contains(&node.label().trim().to_lowercase().as_str())
}
fn attribute<'a>(node: &'a NodeRecord, key: &str) -> Option<&'a str> {
    match key {
        "source_file" => node.source_file(),
        _ => None,
    }
}
fn edge_string(edge: &EdgeRecord, key: &str) -> String {
    edge.string(key)
}
fn defaulted_edge(edge: &EdgeRecord, key: &str, default: &str) -> String {
    let value = edge.string(key);
    if value.is_empty() {
        default.to_owned()
    } else {
        value
    }
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
