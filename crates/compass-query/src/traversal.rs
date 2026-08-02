use std::collections::{BTreeSet, HashMap, HashSet, VecDeque};

use compass_model::{Graph, NodeIndex};
use serde_json::{Map, Value};

use crate::score::{find_exact_nodes, find_node, pick_scored_endpoint, pick_seeds, score_nodes};
use crate::text::{infer_context_filters, normalize_context_filters, query_terms, sanitize_label};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TraversalMode {
    Bfs,
    Dfs,
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
    let terms = query_terms(question);
    let scores = score_nodes(graph, &terms, true);
    let seeds = pick_seeds(graph, &scores, 3, 0.2);
    if seeds.is_empty() {
        return "No matching nodes found.".to_owned();
    }
    let normalized = normalize_context_filters(explicit_contexts);
    let (contexts, source) = if normalized.is_empty() {
        let inferred = infer_context_filters(question);
        let source = (!inferred.is_empty()).then_some("heuristic");
        (inferred, source)
    } else {
        (normalized, Some("explicit"))
    };
    let filtered = graph.with_edge_contexts(&contexts);
    let (nodes, edges) = match mode {
        TraversalMode::Bfs => bfs(&filtered, &seeds, depth),
        TraversalMode::Dfs => dfs(&filtered, &seeds, depth),
    };
    let labels = seeds
        .iter()
        .map(|&node| format!("'{}'", graph.node(node).label()))
        .collect::<Vec<_>>()
        .join(", ");
    let mut header = vec![
        format!("Traversal: {} depth={depth}", mode.upper()),
        format!("Start: [{labels}]"),
    ];
    if !contexts.is_empty() {
        header.push(format!(
            "Context: {} ({})",
            contexts.join(", "),
            source.unwrap_or("explicit")
        ));
    }
    header.push(format!("{} nodes found", nodes.len()));
    format!(
        "{}\n\n{}",
        header.join(" | "),
        render_subgraph(&filtered, &nodes, &edges, token_budget, &seeds, overlay)
    )
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
        return render_ambiguity(graph, label, &matches);
    }
    let Some(&node_index) = matches.first() else {
        return format!("No node matching '{label}' found.");
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
    if !connections.is_empty() {
        lines.push(String::new());
        lines.push(format!("Connections ({}):", connections.len()));
        connections.sort_by(|left, right| {
            let left_source_backed = !graph.node(left.1).string("source_file").is_empty();
            let right_source_backed = !graph.node(right.1).string("source_file").is_empty();
            right_source_backed
                .cmp(&left_source_backed)
                .then_with(|| graph.degree(right.1).cmp(&graph.degree(left.1)))
                .then_with(|| graph.node(left.1).id.cmp(&graph.node(right.1).id))
        });
        for (outgoing, neighbor, edge_index) in connections.iter().take(20) {
            let edge = graph.edge(*edge_index);
            let site = formatted_site(&edge.string("source_file"), &edge.string("source_location"));
            lines.push(format!(
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
            ));
        }
        if connections.len() > 20 {
            lines.push(format!("  ... and {} more", connections.len() - 20));
        }
    }
    lines.join("\n")
}

fn render_ambiguity(graph: &Graph, label: &str, matches: &[NodeIndex]) -> String {
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
    let mut lines = vec![format!(
        "Ambiguous: '{label}' matches {}{qualifier} nodes.",
        matches.len()
    )];
    for index in matches.iter().take(20) {
        let node = graph.node(*index);
        let source_file = node.string("source_file");
        let source_location = node.string("source_location");
        let source = match (source_file.is_empty(), source_location.is_empty()) {
            (true, true) => String::new(),
            (false, true) => source_file,
            (true, false) => source_location,
            (false, false) => format!("{source_file} {source_location}"),
        };
        let wiring = formatted_site(&node.string("wiring_file"), &node.string("wiring_location"));
        let site = if source.is_empty() { wiring } else { source };
        if site.is_empty() {
            lines.push(format!("  {}", node.label()));
        } else {
            lines.push(format!("  {site}"));
        }
        lines.push(format!("    id: {}", node.id));
    }
    if matches.len() > 20 {
        lines.push(format!("  ... and {} more", matches.len() - 20));
    }
    lines.push("Retry with the full node ID.".to_owned());
    lines.join("\n")
}

fn bfs(
    graph: &Graph,
    starts: &[NodeIndex],
    depth: usize,
) -> (HashSet<NodeIndex>, Vec<(NodeIndex, NodeIndex)>) {
    let threshold = hub_threshold(graph);
    let seeds = starts.iter().copied().collect::<HashSet<_>>();
    let mut visited = seeds.clone();
    let mut frontier = starts.iter().copied().collect::<BTreeSet<_>>();
    let mut edges = Vec::new();
    for _ in 0..depth {
        let mut next = BTreeSet::new();
        for node in frontier {
            if !seeds.contains(&node) && graph.degree(node) >= threshold {
                continue;
            }
            for neighbor in graph.successors(node) {
                if !visited.contains(&neighbor) {
                    next.insert(neighbor);
                    edges.push((node, neighbor));
                }
            }
        }
        visited.extend(next.iter().copied());
        frontier = next;
    }
    (visited, edges)
}

fn dfs(
    graph: &Graph,
    starts: &[NodeIndex],
    depth: usize,
) -> (HashSet<NodeIndex>, Vec<(NodeIndex, NodeIndex)>) {
    let threshold = hub_threshold(graph);
    let seeds = starts.iter().copied().collect::<HashSet<_>>();
    let mut visited = HashSet::new();
    let mut edges = Vec::new();
    let mut stack = starts
        .iter()
        .rev()
        .map(|node| (*node, 0_usize))
        .collect::<Vec<_>>();
    while let Some((node, current_depth)) = stack.pop() {
        if visited.contains(&node) || current_depth > depth {
            continue;
        }
        visited.insert(node);
        if !seeds.contains(&node) && graph.degree(node) >= threshold {
            continue;
        }
        for neighbor in graph.successors(node) {
            if !visited.contains(&neighbor) {
                stack.push((neighbor, current_depth + 1));
                edges.push((node, neighbor));
            }
        }
    }
    (visited, edges)
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

fn render_subgraph(
    graph: &Graph,
    nodes: &HashSet<NodeIndex>,
    edges: &[(NodeIndex, NodeIndex)],
    token_budget: usize,
    seeds: &[NodeIndex],
    overlay: &HashMap<String, Map<String, Value>>,
) -> String {
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
    truncate_to_budget(&lines, token_budget)
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

fn truncate_to_budget(lines: &[String], token_budget: usize) -> String {
    let budget = token_budget.saturating_mul(3);
    let output = lines.join("\n");
    if output.chars().count() <= budget {
        return output;
    }
    let prefix = output.chars().take(budget).collect::<String>();
    let cut = prefix.rfind('\n').unwrap_or(prefix.len());
    let visible = &prefix[..cut];
    let total_nodes = lines
        .iter()
        .filter(|line| line.starts_with("NODE "))
        .count();
    let shown_nodes = visible
        .lines()
        .filter(|line| line.starts_with("NODE "))
        .count();
    format!(
        "{visible}\n... (truncated — {} more nodes cut by ~{token_budget}-token budget. Narrow with context_filter=['call'] or use get_node for a specific symbol)",
        total_nodes.saturating_sub(shown_nodes)
    )
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
