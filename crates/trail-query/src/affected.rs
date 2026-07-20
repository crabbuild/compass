use std::collections::{HashSet, VecDeque};
use std::path::Path;

use trail_model::{Graph, NodeIndex};
use unicode_normalization::UnicodeNormalization;

pub const DEFAULT_AFFECTED_RELATIONS: &[&str] = &[
    "calls",
    "indirect_call",
    "references",
    "imports",
    "imports_from",
    "re_exports",
    "inherits",
    "extends",
    "implements",
    "uses",
    "mixes_in",
    "embeds",
];

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AffectedHit {
    pub node: NodeIndex,
    pub depth: usize,
    pub relation: String,
}

#[must_use]
pub fn resolve_seed(graph: &Graph, query: &str) -> Option<NodeIndex> {
    let trimmed = query.trim_end_matches(['/', '\\']);
    let query = if trimmed.is_empty() { query } else { trimmed };
    if let Some(node) = graph.node_index(query) {
        return Some(node);
    }
    let normalized = normalize_label(query);
    let exact = graph
        .nodes()
        .filter(|(_, node)| normalize_label(node.label()) == normalized)
        .map(|(index, _)| index)
        .collect::<Vec<_>>();
    if exact.len() == 1 {
        return exact.first().copied();
    }
    let bare = bare_name(&normalized);
    let bare_matches = graph
        .nodes()
        .filter(|(_, node)| bare_name(node.label()) == bare)
        .map(|(index, _)| index)
        .collect::<Vec<_>>();
    if bare_matches.len() == 1 {
        return bare_matches.first().copied();
    }
    let source_matches = graph
        .nodes()
        .filter(|(_, node)| normalize_label(&node.string("source_file")) == normalized)
        .map(|(index, _)| index)
        .collect::<Vec<_>>();
    if source_matches.len() == 1 {
        return source_matches.first().copied();
    }
    if let Some(node) = prefer_file_node(graph, &source_matches, query) {
        return Some(node);
    }
    let contains = graph
        .nodes()
        .filter(|(_, node)| normalize_label(node.label()).contains(&normalized))
        .map(|(index, _)| index)
        .collect::<Vec<_>>();
    (contains.len() == 1).then(|| contains[0])
}

#[must_use]
pub fn affected_nodes(
    graph: &Graph,
    seed: NodeIndex,
    relations: &[String],
    depth: usize,
) -> Vec<AffectedHit> {
    let relation_set = relations.iter().map(String::as_str).collect::<HashSet<_>>();
    let mut seen = HashSet::from([seed]);
    let mut queue = VecDeque::from([(seed, 0_usize)]);
    let mut hits = Vec::new();
    for edge_index in graph.outgoing_edges(seed) {
        let edge = graph.edge(edge_index);
        if !["method", "contains"].contains(&edge.string("relation").as_str()) {
            continue;
        }
        if let Some(member) = graph.node_index(&edge.target)
            && seen.insert(member)
        {
            queue.push_back((member, 0));
        }
    }
    while let Some((current, current_depth)) = queue.pop_front() {
        if current_depth >= depth {
            continue;
        }
        for edge_index in graph.incoming_edges(current) {
            let edge = graph.edge(edge_index);
            let relation = edge.string("relation");
            if !relation_set.contains(relation.as_str()) {
                continue;
            }
            let Some(source) = graph.node_index(&edge.source) else {
                continue;
            };
            if !seen.insert(source) {
                continue;
            }
            hits.push(AffectedHit {
                node: source,
                depth: current_depth + 1,
                relation,
            });
            queue.push_back((source, current_depth + 1));
        }
    }
    hits
}

#[must_use]
pub fn format_affected(graph: &Graph, query: &str, relations: &[String], depth: usize) -> String {
    let Some(seed) = resolve_seed(graph, query) else {
        return format!("No unique node match for {query}");
    };
    let hits = affected_nodes(graph, seed, relations, depth);
    let mut lines = vec![
        format!("Affected nodes for {}", graph.node(seed).label()),
        format!("Relations: {}", relations.join(", ")),
        format!("Depth: {depth}"),
    ];
    if hits.is_empty() {
        lines.push("No affected nodes found.".to_owned());
    } else {
        for hit in hits {
            let node = graph.node(hit.node);
            let source = node.string("source_file");
            let source = if source.is_empty() { "-" } else { &source };
            let location = node.string("source_location");
            let location = if location.is_empty() {
                source.to_owned()
            } else {
                format!("{source}:{location}")
            };
            lines.push(format!("- {} [{}] {location}", node.label(), hit.relation));
        }
    }
    lines.join("\n")
}

fn prefer_file_node(graph: &Graph, nodes: &[NodeIndex], query: &str) -> Option<NodeIndex> {
    if nodes.is_empty() {
        return None;
    }
    let basename = Path::new(query)
        .file_name()
        .and_then(|name| name.to_str())
        .map_or_else(String::new, normalize_label);
    let exact = nodes
        .iter()
        .copied()
        .filter(|&node| {
            graph.node(node).string("source_location") == "L1"
                && normalize_label(graph.node(node).label()) == basename
        })
        .collect::<Vec<_>>();
    if exact.len() == 1 {
        return exact.first().copied();
    }
    let level_one = nodes
        .iter()
        .copied()
        .filter(|&node| graph.node(node).string("source_location") == "L1")
        .collect::<Vec<_>>();
    if level_one.len() == 1 {
        return level_one.first().copied();
    }
    let basename_nodes = nodes
        .iter()
        .copied()
        .filter(|&node| normalize_label(graph.node(node).label()) == basename)
        .collect::<Vec<_>>();
    (basename_nodes.len() == 1).then(|| basename_nodes[0])
}

fn normalize_label(label: &str) -> String {
    label.nfc().collect::<String>().to_lowercase()
}

fn bare_name(label: &str) -> String {
    let normalized = normalize_label(label);
    normalized
        .strip_suffix("()")
        .unwrap_or(&normalized)
        .to_owned()
}
