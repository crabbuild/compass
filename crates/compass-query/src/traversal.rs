use std::collections::{BTreeSet, HashMap, HashSet, VecDeque};

use compass_model::query_contract::{
    DiscoveryLimits, MAX_DISCOVERY_EDGES, MAX_DISCOVERY_EXPANDED_RELATIONSHIPS, MAX_DISCOVERY_NODES,
};
use compass_model::{Graph, NodeIndex};
use serde_json::{Map, Value};
use thiserror::Error;

use crate::score::{find_exact_nodes, find_node, pick_scored_endpoint, pick_seeds, score_nodes};
use crate::text::{infer_context_filters, normalize_context_filters, query_terms, sanitize_label};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TraversalMode {
    Bfs,
    Dfs,
}

pub const DEFAULT_TEXT_TOKEN_BUDGET: usize = 2_000;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct TextPageOptions {
    pub token_budget: usize,
    pub page: usize,
}

#[derive(Clone, Debug)]
struct NaturalQueryAssembly {
    seeds: Vec<NodeIndex>,
    nodes: HashSet<NodeIndex>,
    edges: Vec<(NodeIndex, NodeIndex)>,
    contexts: Vec<String>,
    context_source: Option<&'static str>,
    equally_ranked_seed_candidates: usize,
    expanded_relationships: u64,
    truncated: bool,
}

#[derive(Clone, Debug)]
struct TraversalSelection {
    nodes: HashSet<NodeIndex>,
    edges: Vec<(NodeIndex, NodeIndex)>,
    expanded_relationships: u64,
    truncated: bool,
}

#[derive(Clone, Debug, Eq, Error, PartialEq)]
pub enum TextPaginationError {
    #[error("token budget must be greater than zero")]
    ZeroBudget,
    #[error("page must be greater than zero")]
    ZeroPage,
    #[error("page {requested} exceeds the last available page {last}")]
    PageOutOfRange { requested: usize, last: usize },
}

impl TraversalMode {
    fn upper(self) -> &'static str {
        match self {
            Self::Bfs => "BFS",
            Self::Dfs => "DFS",
        }
    }
}

#[must_use]
pub fn query_graph_text(
    graph: &Graph,
    question: &str,
    mode: TraversalMode,
    depth: usize,
    token_budget: usize,
    explicit_contexts: &[String],
    overlay: &HashMap<String, Map<String, Value>>,
) -> String {
    match query_graph_text_page(
        graph,
        question,
        mode,
        depth,
        TextPageOptions {
            token_budget,
            page: 1,
        },
        explicit_contexts,
        overlay,
    ) {
        Ok(output) => output,
        Err(error) => format!("Query output error: {error}."),
    }
}

pub fn query_graph_text_page(
    graph: &Graph,
    question: &str,
    mode: TraversalMode,
    depth: usize,
    options: TextPageOptions,
    explicit_contexts: &[String],
    overlay: &HashMap<String, Map<String, Value>>,
) -> Result<String, TextPaginationError> {
    let TextPageOptions { token_budget, page } = options;
    validate_pagination(token_budget, page)?;
    let Some(assembly) = assemble_natural_query(graph, question, mode, depth, explicit_contexts)
    else {
        return if page == 1 {
            Ok("No matching nodes found.".to_owned())
        } else {
            Err(TextPaginationError::PageOutOfRange {
                requested: page,
                last: 1,
            })
        };
    };
    let filtered = graph.with_edge_contexts(&assembly.contexts);
    let labels = assembly
        .seeds
        .iter()
        .map(|&node| format!("'{}'", graph.node(node).label()))
        .collect::<Vec<_>>()
        .join(", ");
    let mut header = vec![
        format!("Traversal: {} depth={depth}", mode.upper()),
        "Direction: outgoing (compatibility)".to_owned(),
        format!(
            "Ambiguity: {} equally ranked top candidate(s)",
            assembly.equally_ranked_seed_candidates
        ),
        format!("Start: [{labels}]"),
    ];
    if !assembly.contexts.is_empty() {
        header.push(format!(
            "Context: {} ({})",
            assembly.contexts.join(", "),
            assembly.context_source.unwrap_or("explicit")
        ));
    }
    header.push(if assembly.truncated {
        format!(
            "Completion: bounded after {} relationship expansions",
            assembly.expanded_relationships
        )
    } else {
        "Completion: complete".to_owned()
    });
    header.push(format!("{} nodes found", assembly.nodes.len()));
    let header = header.join(" | ");
    let lines = render_subgraph_lines(
        &filtered,
        &assembly.nodes,
        &assembly.edges,
        &assembly.seeds,
        overlay,
    );
    let page = render_paginated_lines(
        &lines,
        token_budget,
        page,
        header.chars().count().saturating_add(2),
        "facts",
    )?;
    Ok(format!("{header}\n\n{page}"))
}

fn assemble_natural_query(
    graph: &Graph,
    question: &str,
    mode: TraversalMode,
    depth: usize,
    explicit_contexts: &[String],
) -> Option<NaturalQueryAssembly> {
    let terms = query_terms(question);
    let scores = score_nodes(graph, &terms, true);
    let max_seeds = usize::try_from(DiscoveryLimits::default().max_seeds).unwrap_or(usize::MAX);
    let equally_ranked_seed_candidates = scores.ranked.first().map_or(0, |first| {
        scores
            .ranked
            .iter()
            .take_while(|candidate| candidate.score.total_cmp(&first.score).is_eq())
            .count()
    });
    let mut seeds = pick_seeds(graph, &scores, max_seeds, 0.2);
    seeds.truncate(max_seeds);
    if seeds.is_empty() {
        return None;
    }
    let normalized = normalize_context_filters(explicit_contexts);
    let (contexts, context_source) = if normalized.is_empty() {
        let inferred = infer_context_filters(question);
        let source = (!inferred.is_empty()).then_some("heuristic");
        (inferred, source)
    } else {
        (normalized, Some("explicit"))
    };
    let filtered = graph.with_edge_contexts(&contexts);
    let selection = match mode {
        TraversalMode::Bfs => bfs(&filtered, &seeds, depth),
        TraversalMode::Dfs => dfs(&filtered, &seeds, depth),
    };
    Some(NaturalQueryAssembly {
        seeds,
        nodes: selection.nodes,
        edges: selection.edges,
        contexts,
        context_source,
        equally_ranked_seed_candidates,
        expanded_relationships: selection.expanded_relationships,
        truncated: selection.truncated,
    })
}

pub fn render_shortest_path(
    graph: &Graph,
    source_query: &str,
    target_query: &str,
) -> Result<String, String> {
    let source_scores = score_nodes(
        graph,
        &source_query
            .split_whitespace()
            .map(str::to_lowercase)
            .collect::<Vec<_>>(),
        false,
    );
    let target_scores = score_nodes(
        graph,
        &target_query
            .split_whitespace()
            .map(str::to_lowercase)
            .collect::<Vec<_>>(),
        false,
    );
    if source_scores.ranked.is_empty() {
        return Err(format!("No node matching '{source_query}' found."));
    }
    if target_scores.ranked.is_empty() {
        return Err(format!("No node matching '{target_query}' found."));
    }
    let source = pick_scored_endpoint(graph, &source_scores.ranked, source_query);
    let target = pick_scored_endpoint(graph, &target_scores.ranked, target_query);
    if source == target {
        return Err(format!(
            "'{source_query}' and '{target_query}' both resolved to the same node '{}'. Use a more specific label or the exact node ID.",
            graph.node(source).id
        ));
    }
    let Some(path) = shortest_path_undirected(graph, source, target) else {
        return Ok(format!(
            "No path found between '{source_query}' and '{target_query}'."
        ));
    };
    let mut segments = vec![graph.node(path[0]).label().to_owned()];
    for pair in path.windows(2) {
        let left = pair[0];
        let right = pair[1];
        if let Some(edge_index) = graph.edge_between(left, right) {
            let edge = graph.edge(edge_index);
            let confidence = edge.string("confidence");
            let suffix = if confidence.is_empty() {
                String::new()
            } else {
                format!(" [{confidence}]")
            };
            segments.push(format!(
                "--{}{}--> {}",
                edge.string("relation"),
                suffix,
                graph.node(right).label()
            ));
        } else if let Some(edge_index) = graph.edge_between(right, left) {
            let edge = graph.edge(edge_index);
            let confidence = edge.string("confidence");
            let suffix = if confidence.is_empty() {
                String::new()
            } else {
                format!(" [{confidence}]")
            };
            segments.push(format!(
                "<--{}{}-- {}",
                edge.string("relation"),
                suffix,
                graph.node(right).label()
            ));
        }
    }
    Ok(format!(
        "Shortest path ({} hops):\n  {}",
        path.len() - 1,
        segments.join(" ")
    ))
}

#[must_use]
pub fn render_explanation(
    graph: &Graph,
    label: &str,
    overlay: &HashMap<String, Map<String, Value>>,
) -> String {
    match render_explanation_page(graph, label, DEFAULT_TEXT_TOKEN_BUDGET, 1, overlay) {
        Ok(output) => output,
        Err(error) => format!("Explanation output error: {error}."),
    }
}

pub fn render_explanation_page(
    graph: &Graph,
    label: &str,
    token_budget: usize,
    page: usize,
    overlay: &HashMap<String, Map<String, Value>>,
) -> Result<String, TextPaginationError> {
    validate_pagination(token_budget, page)?;
    let exact_matches = find_exact_nodes(graph, label);
    let mut matches = if exact_matches.is_empty() {
        find_node(graph, label)
    } else {
        exact_matches
    };
    let source_backed = matches
        .iter()
        .copied()
        .filter(|index| !graph.node(*index).string("source_file").is_empty())
        .collect::<Vec<_>>();
    if !source_backed.is_empty() {
        matches = source_backed;
    }
    if matches.len() > 1 {
        return render_ambiguity_page(graph, label, &matches, token_budget, page);
    }
    let Some(&node_index) = matches.first() else {
        return if page == 1 {
            Ok(format!("No node matching '{label}' found."))
        } else {
            Err(TextPaginationError::PageOutOfRange {
                requested: page,
                last: 1,
            })
        };
    };
    let node = graph.node(node_index);
    let mut lines = vec![
        format!("Node: {}", node.label()),
        format!("  ID:        {}", node.id),
    ];
    let source = node.string("source_file");
    let location = node.string("source_location");
    lines.push(
        format!("  Source:    {source} {location}")
            .trim_end()
            .to_owned(),
    );
    let wiring_file = node.string("wiring_file");
    let wiring_location = node.string("wiring_location");
    if source.is_empty() && (!wiring_file.is_empty() || !wiring_location.is_empty()) {
        lines.push(
            format!("  Wiring:    {wiring_file} {wiring_location}")
                .trim_end()
                .to_owned(),
        );
    }
    lines.push(format!("  Type:      {}", rendered_file_type(node)));
    let community_name = node.string("community_name");
    let community = if community_name.is_empty() {
        node.string("community")
    } else {
        community_name
    };
    lines.push(format!("  Community: {community}"));
    if let Some(entry) = overlay.get(&node.id) {
        let status = json_string(entry.get("status"));
        let uses = json_string(entry.get("uses"));
        let stale = entry.get("stale").and_then(Value::as_bool).unwrap_or(false);
        let mut lesson = if status == "contested" {
            format!(
                "  Lesson: contested (useful {uses} / dead-end {})",
                json_string(entry.get("neg"))
            )
        } else if status == "preferred" {
            format!(
                "  Lesson: preferred source (start here) — {uses} useful, score={}",
                json_string(entry.get("score"))
            )
        } else {
            format!(
                "  Lesson: {} — {uses} useful, score={}",
                if status.is_empty() {
                    "tentative"
                } else {
                    &status
                },
                json_string(entry.get("score"))
            )
        };
        if stale {
            lesson.push_str(" [code changed since — re-verify]");
        }
        lines.push(lesson);
    }
    lines.push(format!("  Degree:    {}", graph.degree(node_index)));
    let mut connections = Vec::new();
    let mut seen_connections = HashSet::new();
    for edge in graph.outgoing_edges(node_index) {
        if let Some(neighbor) = graph.node_index(&graph.edge(edge).target)
            && seen_connections.insert((true, neighbor))
        {
            connections.push((true, neighbor, edge));
        }
    }
    for edge in graph.incoming_edges(node_index) {
        if let Some(neighbor) = graph.node_index(&graph.edge(edge).source)
            && seen_connections.insert((false, neighbor))
        {
            connections.push((false, neighbor, edge));
        }
    }
    if connections.is_empty() {
        return if page == 1 {
            Ok(lines.join("\n"))
        } else {
            Err(TextPaginationError::PageOutOfRange {
                requested: page,
                last: 1,
            })
        };
    }
    connections.sort_by(|left, right| {
        let left_source_backed = !graph.node(left.1).string("source_file").is_empty();
        let right_source_backed = !graph.node(right.1).string("source_file").is_empty();
        right_source_backed
            .cmp(&left_source_backed)
            .then_with(|| graph.degree(right.1).cmp(&graph.degree(left.1)))
            .then_with(|| graph.node(left.1).id.cmp(&graph.node(right.1).id))
    });
    let connection_lines = connections
        .iter()
        .map(|(outgoing, neighbor, edge_index)| {
            let edge = graph.edge(*edge_index);
            let site = formatted_site(&edge.string("source_file"), &edge.string("source_location"));
            vec![format!(
                "  {} {} [{}] [{}]{}",
                if *outgoing { "-->" } else { "<--" },
                graph.node(*neighbor).label(),
                edge.string("relation"),
                edge.string("confidence"),
                if site.is_empty() {
                    String::new()
                } else {
                    format!(" {site}")
                }
            )]
        })
        .collect::<Vec<_>>();
    lines.push(String::new());
    lines.push(format!("Connections ({}):", connection_lines.len()));
    let fixed = lines.join("\n");
    let rendered = render_paginated_groups(
        &connection_lines,
        token_budget,
        page,
        fixed.chars().count().saturating_add(1),
        "connections",
    )?;
    Ok(format!("{fixed}\n{rendered}"))
}

fn render_ambiguity_page(
    graph: &Graph,
    label: &str,
    matches: &[NodeIndex],
    token_budget: usize,
    page: usize,
) -> Result<String, TextPaginationError> {
    let mut matches = matches.to_vec();
    matches.sort_by(|left, right| {
        let left_node = graph.node(*left);
        let right_node = graph.node(*right);
        left_node
            .string("source_file")
            .cmp(&right_node.string("source_file"))
            .then_with(|| {
                left_node
                    .string("source_location")
                    .cmp(&right_node.string("source_location"))
            })
            .then_with(|| left_node.id.cmp(&right_node.id))
    });
    let all_source_backed = matches
        .iter()
        .all(|index| !graph.node(*index).string("source_file").is_empty());
    let qualifier = if all_source_backed {
        " source-backed"
    } else {
        ""
    };
    let header = format!(
        "Ambiguous: '{label}' matches {}{qualifier} nodes.",
        matches.len()
    );
    let groups = matches
        .iter()
        .map(|index| {
            let node = graph.node(*index);
            let source_file = node.string("source_file");
            let source_location = node.string("source_location");
            let source = match (source_file.is_empty(), source_location.is_empty()) {
                (true, true) => String::new(),
                (false, true) => source_file,
                (true, false) => source_location,
                (false, false) => format!("{source_file} {source_location}"),
            };
            let wiring =
                formatted_site(&node.string("wiring_file"), &node.string("wiring_location"));
            let site = if source.is_empty() { wiring } else { source };
            let summary = if site.is_empty() {
                format!("  {}", node.label())
            } else {
                format!("  {site}")
            };
            vec![summary, format!("    id: {}", node.id)]
        })
        .collect::<Vec<_>>();
    let footer = "Retry with the full node ID.";
    let overhead = header
        .chars()
        .count()
        .saturating_add(footer.chars().count())
        .saturating_add(2);
    let rendered = render_paginated_groups(&groups, token_budget, page, overhead, "matches")?;
    Ok(format!("{header}\n{rendered}\n{footer}"))
}

fn bfs(graph: &Graph, starts: &[NodeIndex], depth: usize) -> TraversalSelection {
    let threshold = hub_threshold(graph);
    let seeds = starts.iter().copied().collect::<HashSet<_>>();
    let mut visited = seeds.clone();
    let mut frontier = starts.iter().copied().collect::<BTreeSet<_>>();
    let mut edges = Vec::new();
    let mut expanded_relationships = 0_u64;
    let mut truncated = false;
    let max_nodes = usize::try_from(MAX_DISCOVERY_NODES).unwrap_or(usize::MAX);
    let max_edges = usize::try_from(MAX_DISCOVERY_EDGES).unwrap_or(usize::MAX);
    for _ in 0..depth {
        let mut next = BTreeSet::new();
        for node in frontier {
            if !seeds.contains(&node) && graph.degree(node) >= threshold {
                continue;
            }
            for neighbor in graph.successors(node) {
                if expanded_relationships >= MAX_DISCOVERY_EXPANDED_RELATIONSHIPS {
                    truncated = true;
                    break;
                }
                expanded_relationships = expanded_relationships.saturating_add(1);
                if !visited.contains(&neighbor) {
                    if visited.len().saturating_add(next.len()) >= max_nodes
                        || edges.len() >= max_edges
                    {
                        truncated = true;
                        continue;
                    }
                    next.insert(neighbor);
                    edges.push((node, neighbor));
                }
            }
        }
        visited.extend(next.iter().copied());
        frontier = next;
    }
    TraversalSelection {
        nodes: visited,
        edges,
        expanded_relationships,
        truncated,
    }
}

fn dfs(graph: &Graph, starts: &[NodeIndex], depth: usize) -> TraversalSelection {
    let threshold = hub_threshold(graph);
    let seeds = starts.iter().copied().collect::<HashSet<_>>();
    let mut visited = HashSet::new();
    let mut discovered = seeds.clone();
    let mut edges = Vec::new();
    let mut expanded_relationships = 0_u64;
    let mut truncated = false;
    let max_nodes = usize::try_from(MAX_DISCOVERY_NODES).unwrap_or(usize::MAX);
    let max_edges = usize::try_from(MAX_DISCOVERY_EDGES).unwrap_or(usize::MAX);
    let mut stack = starts
        .iter()
        .rev()
        .map(|node| (*node, 0_usize))
        .collect::<Vec<_>>();
    while let Some((node, current_depth)) = stack.pop() {
        if visited.contains(&node) || current_depth > depth {
            continue;
        }
        if visited.len() >= max_nodes {
            truncated = true;
            continue;
        }
        visited.insert(node);
        if current_depth >= depth || (!seeds.contains(&node) && graph.degree(node) >= threshold) {
            continue;
        }
        for neighbor in graph.successors(node) {
            if expanded_relationships >= MAX_DISCOVERY_EXPANDED_RELATIONSHIPS {
                truncated = true;
                break;
            }
            expanded_relationships = expanded_relationships.saturating_add(1);
            if !visited.contains(&neighbor) && !discovered.contains(&neighbor) {
                if edges.len() >= max_edges || discovered.len() >= max_nodes {
                    truncated = true;
                    continue;
                }
                discovered.insert(neighbor);
                stack.push((neighbor, current_depth + 1));
                edges.push((node, neighbor));
            }
        }
    }
    TraversalSelection {
        nodes: visited,
        edges,
        expanded_relationships,
        truncated,
    }
}

fn hub_threshold(graph: &Graph) -> usize {
    let mut degrees = graph
        .nodes()
        .map(|(node, _)| graph.degree(node))
        .collect::<Vec<_>>();
    if degrees.is_empty() {
        return 50;
    }
    degrees.sort_unstable();
    let index = ((degrees.len() as f64) * 0.99) as usize;
    degrees[index.min(degrees.len() - 1)].max(50)
}

fn render_subgraph_lines(
    graph: &Graph,
    nodes: &HashSet<NodeIndex>,
    edges: &[(NodeIndex, NodeIndex)],
    seeds: &[NodeIndex],
    overlay: &HashMap<String, Map<String, Value>>,
) -> Vec<String> {
    let seed_set = seeds.iter().copied().collect::<HashSet<_>>();
    let mut ordered = seeds
        .iter()
        .copied()
        .filter(|node| nodes.contains(node))
        .collect::<Vec<_>>();
    let mut remainder = nodes
        .iter()
        .copied()
        .filter(|node| !seed_set.contains(node))
        .collect::<Vec<_>>();
    remainder.sort_by(|left, right| {
        let left_source_backed = !graph.node(*left).string("source_file").is_empty();
        let right_source_backed = !graph.node(*right).string("source_file").is_empty();
        right_source_backed
            .cmp(&left_source_backed)
            .then_with(|| graph.degree(*right).cmp(&graph.degree(*left)))
            .then_with(|| graph.node(*left).id.cmp(&graph.node(*right).id))
    });
    ordered.extend(remainder);
    let mut lines = Vec::new();
    for node_index in ordered {
        let node = graph.node(node_index);
        let community_name = node.string("community_name");
        let community = if community_name.is_empty() {
            node.string("community")
        } else {
            community_name
        };
        let learning = overlay.get(&node.id).and_then(|entry| {
            let status = json_string(entry.get("status"));
            (!status.is_empty()).then(|| {
                let stale = entry.get("stale").and_then(Value::as_bool).unwrap_or(false);
                format!(" learning={status}{}", if stale { ":stale" } else { "" })
            })
        });
        let source = node.string("source_file");
        let location = node.string("source_location");
        let wiring = contextual_wiring_site(graph, node_index, edges).unwrap_or_else(|| {
            formatted_site(&node.string("wiring_file"), &node.string("wiring_location"))
        });
        let wiring = if source.is_empty() && !wiring.is_empty() {
            format!(" wiring={}", sanitize_label(&wiring))
        } else {
            String::new()
        };
        lines.push(format!(
            "NODE {} [src={} loc={}{} community={}{}]",
            sanitize_label(node.label()),
            sanitize_label(&source),
            sanitize_label(&location),
            wiring,
            sanitize_label(&community),
            learning.unwrap_or_default()
        ));
    }
    for &(source, target) in edges {
        if !nodes.contains(&source) || !nodes.contains(&target) {
            continue;
        }
        let Some(edge_index) = graph.edge_between(source, target) else {
            continue;
        };
        let edge = graph.edge(edge_index);
        let context = edge.string("context");
        let context = if context.is_empty() {
            String::new()
        } else {
            format!(" context={}", sanitize_label(&context))
        };
        let site = formatted_site(&edge.string("source_file"), &edge.string("source_location"));
        let site = if site.is_empty() {
            String::new()
        } else {
            format!(" at={}", sanitize_label(&site))
        };
        lines.push(format!(
            "EDGE {} --{} [{}{}]--> {}{}",
            sanitize_label(graph.node(source).label()),
            sanitize_label(&edge.string("relation")),
            sanitize_label(&edge.string("confidence")),
            context,
            sanitize_label(graph.node(target).label()),
            site
        ));
    }
    lines
}

fn formatted_site(file: &str, location: &str) -> String {
    match (file.is_empty(), location.is_empty()) {
        (true, true) => String::new(),
        (false, true) => file.to_owned(),
        (true, false) => location.to_owned(),
        (false, false) => format!("{file}:{location}"),
    }
}

fn rendered_file_type(node: &compass_model::NodeRecord) -> String {
    let stored = node.string("file_type");
    if !stored.is_empty() {
        return stored;
    }
    node.attributes
        .get("kind")
        .and_then(Value::as_str)
        .map_or_else(String::new, |kind| {
            if kind == "resource" {
                "document".to_owned()
            } else {
                "code".to_owned()
            }
        })
}

fn contextual_wiring_site(
    graph: &Graph,
    node: NodeIndex,
    edges: &[(NodeIndex, NodeIndex)],
) -> Option<String> {
    edges
        .iter()
        .filter(|(source, target)| *source == node || *target == node)
        .filter_map(|(source, target)| graph.edge_between(*source, *target))
        .map(|edge| graph.edge(edge))
        .map(|edge| formatted_site(&edge.string("source_file"), &edge.string("source_location")))
        .filter(|site| !site.is_empty())
        .min()
}

fn validate_pagination(token_budget: usize, page: usize) -> Result<(), TextPaginationError> {
    if token_budget == 0 {
        return Err(TextPaginationError::ZeroBudget);
    }
    if page == 0 {
        return Err(TextPaginationError::ZeroPage);
    }
    Ok(())
}

fn render_paginated_lines(
    lines: &[String],
    token_budget: usize,
    page: usize,
    overhead_chars: usize,
    item_label: &str,
) -> Result<String, TextPaginationError> {
    let groups = lines
        .iter()
        .cloned()
        .map(|line| vec![line])
        .collect::<Vec<_>>();
    render_paginated_groups(&groups, token_budget, page, overhead_chars, item_label)
}

fn render_paginated_groups(
    groups: &[Vec<String>],
    token_budget: usize,
    page: usize,
    overhead_chars: usize,
    item_label: &str,
) -> Result<String, TextPaginationError> {
    validate_pagination(token_budget, page)?;
    let capacity = token_budget
        .saturating_mul(3)
        .saturating_sub(overhead_chars)
        .max(1);
    let mut ranges = Vec::new();
    let mut start = 0;
    let mut used = 0_usize;
    for (index, group) in groups.iter().enumerate() {
        let group_chars = group
            .iter()
            .map(|line| line.chars().count())
            .sum::<usize>()
            .saturating_add(group.len().saturating_sub(1));
        let separator = usize::from(index > start);
        if index > start && used.saturating_add(separator).saturating_add(group_chars) > capacity {
            ranges.push(start..index);
            start = index;
            used = group_chars;
        } else {
            used = used.saturating_add(separator).saturating_add(group_chars);
        }
    }
    if start < groups.len() {
        ranges.push(start..groups.len());
    }
    if ranges.is_empty() {
        ranges.push(0..0);
    }
    let total_pages = ranges.len();
    let Some(range) = ranges.get(page - 1) else {
        return Err(TextPaginationError::PageOutOfRange {
            requested: page,
            last: total_pages,
        });
    };
    let body = groups[range.clone()]
        .iter()
        .flat_map(|group| group.iter())
        .cloned()
        .collect::<Vec<_>>()
        .join("\n");
    let first = if range.is_empty() { 0 } else { range.start + 1 };
    let last = range.end;
    let previous = page
        .checked_sub(1)
        .filter(|previous| *previous > 0)
        .map_or_else(|| "none".to_owned(), |previous| previous.to_string());
    let next = if page < total_pages {
        (page + 1).to_string()
    } else {
        "none".to_owned()
    };
    let pagination = format!(
        "Pagination: page={page}/{total_pages} {item_label}={first}-{last}/{} budget_tokens=~{token_budget} previous={previous} next={next}",
        groups.len()
    );
    if body.is_empty() {
        Ok(pagination)
    } else {
        Ok(format!("{body}\n{pagination}"))
    }
}

fn shortest_path_undirected(
    graph: &Graph,
    source: NodeIndex,
    target: NodeIndex,
) -> Option<Vec<NodeIndex>> {
    let mut queue = VecDeque::from([source]);
    let mut previous = HashMap::from([(source, source)]);
    while let Some(node) = queue.pop_front() {
        if node == target {
            break;
        }
        for neighbor in graph.successors(node).chain(graph.predecessors(node)) {
            if let std::collections::hash_map::Entry::Vacant(entry) = previous.entry(neighbor) {
                entry.insert(node);
                queue.push_back(neighbor);
            }
        }
    }
    if !previous.contains_key(&target) {
        return None;
    }
    let mut path = vec![target];
    let mut current = target;
    while current != source {
        current = previous[&current];
        path.push(current);
    }
    path.reverse();
    Some(path)
}

fn json_string(value: Option<&Value>) -> String {
    match value {
        None | Some(Value::Null) => String::new(),
        Some(Value::String(text)) => text.clone(),
        Some(Value::Bool(value)) => if *value { "True" } else { "False" }.to_owned(),
        Some(Value::Number(value)) => value.to_string(),
        Some(value) => value.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use compass_model::{Graph, GraphDocument};
    use serde_json::json;

    use super::dfs;

    #[test]
    fn dfs_edges_always_reference_visited_nodes_at_depth_cap()
    -> Result<(), Box<dyn std::error::Error>> {
        let document = serde_json::from_value::<GraphDocument>(json!({
            "directed": true,
            "multigraph": true,
            "graph": {},
            "nodes": [
                {"id": "seed", "label": "seed"},
                {"id": "depth-one", "label": "depth one"},
                {"id": "depth-two", "label": "depth two"}
            ],
            "links": [
                {"source": "seed", "target": "depth-one", "relation": "calls"},
                {"source": "depth-one", "target": "depth-two", "relation": "calls"}
            ]
        }))?;
        let graph = Graph::from_document(document)?;
        let seed = graph
            .node_index("seed")
            .ok_or_else(|| std::io::Error::other("seed must exist in test graph"))?;
        let selection = dfs(&graph, &[seed], 1);
        assert!(selection.edges.iter().all(|(source, target)| {
            selection.nodes.contains(source) && selection.nodes.contains(target)
        }));
        assert_eq!(selection.nodes.len(), 2);
        assert_eq!(selection.edges.len(), 1);
        Ok(())
    }
}
