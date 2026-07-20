use std::collections::{BTreeMap, HashMap, HashSet};
use std::fmt::Write as _;
use std::fs;
use std::path::{Path, PathBuf};

use trail_files::write_text_atomic;
use trail_graph::{Communities, GodNode};
use trail_model::{EdgeRecord, GraphDocument, NodeRecord};

use crate::OutputError;

#[derive(Clone, Debug, Default)]
pub struct WikiOptions<'a> {
    pub community_labels: Option<&'a BTreeMap<usize, String>>,
    pub cohesion: Option<&'a BTreeMap<usize, f64>>,
    pub god_nodes: Option<&'a [GodNode]>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct WikiExport {
    pub articles_written: usize,
    pub stale_nodes_dropped: usize,
    pub communities_written: usize,
}

pub fn export_wiki(
    document: &GraphDocument,
    communities: &Communities,
    output_dir: impl AsRef<Path>,
    options: &WikiOptions<'_>,
) -> Result<WikiExport, OutputError> {
    if communities.is_empty() {
        return Err(OutputError::EmptyWikiCommunities);
    }
    let graph = WikiGraph::new(document);
    let original_total = communities.values().map(Vec::len).sum::<usize>();
    let communities = communities
        .iter()
        .filter_map(|(community, members)| {
            let retained = members
                .iter()
                .filter(|member| graph.nodes.contains_key(member.as_str()))
                .cloned()
                .collect::<Vec<_>>();
            (!retained.is_empty()).then_some((*community, retained))
        })
        .collect::<Communities>();
    let kept_total = communities.values().map(Vec::len).sum::<usize>();
    if communities.is_empty() {
        return Err(OutputError::StaleWikiCommunities);
    }

    let output_dir = output_dir.as_ref();
    fs::create_dir_all(output_dir).map_err(|source| wiki_io(output_dir, source))?;
    for entry in fs::read_dir(output_dir).map_err(|source| wiki_io(output_dir, source))? {
        let entry = entry.map_err(|source| wiki_io(output_dir, source))?;
        let path = entry.path();
        if path.extension().and_then(|value| value.to_str()) == Some("md") {
            fs::remove_file(&path).map_err(|source| wiki_io(&path, source))?;
        }
    }

    let labels = communities
        .keys()
        .map(|community| {
            (
                *community,
                options
                    .community_labels
                    .and_then(|labels| labels.get(community))
                    .cloned()
                    .unwrap_or_else(|| format!("Community {community}")),
            )
        })
        .collect::<BTreeMap<_, _>>();
    let cohesion = options.cohesion.cloned().unwrap_or_default();
    let gods = options.god_nodes.unwrap_or_default();
    let node_community = communities
        .iter()
        .flat_map(|(community, members)| {
            members
                .iter()
                .map(move |member| (member.clone(), *community))
        })
        .collect::<HashMap<_, _>>();

    let mut used_slugs = HashSet::new();
    let mut resolver = HashMap::from([("index".to_owned(), "index".to_owned())]);
    let mut community_slugs = BTreeMap::new();
    for community in communities.keys() {
        let label = &labels[community];
        let slug = unique_slug(&safe_filename(label), &mut used_slugs);
        community_slugs.insert(*community, slug.clone());
        resolver.entry(label.clone()).or_insert(slug);
    }
    let mut god_articles = Vec::new();
    for god in gods {
        if graph.nodes.contains_key(god.id.as_str()) {
            let slug = unique_slug(&safe_filename(&god.label), &mut used_slugs);
            god_articles.push((god, slug.clone()));
            resolver.entry(god.label.clone()).or_insert(slug);
        }
    }

    let mut count = 0;
    for (community, members) in &communities {
        let article = community_article(
            &graph,
            *community,
            members,
            &labels[community],
            &labels,
            cohesion.get(community).copied(),
            &node_community,
            &resolver,
        );
        write_text_atomic(
            output_dir.join(format!("{}.md", community_slugs[community])),
            &article,
        )?;
        count += 1;
    }
    for (god, slug) in &god_articles {
        let article = god_node_article(&graph, god, &labels, &node_community, &resolver);
        write_text_atomic(output_dir.join(format!("{slug}.md")), &article)?;
        count += 1;
    }
    write_text_atomic(
        output_dir.join("index.md"),
        &index_markdown(
            &communities,
            &labels,
            gods,
            document.nodes.len(),
            document.links.len(),
            &resolver,
        ),
    )?;
    Ok(WikiExport {
        articles_written: count,
        stale_nodes_dropped: original_total - kept_total,
        communities_written: communities.len(),
    })
}

struct WikiGraph<'a> {
    directed: bool,
    nodes: HashMap<&'a str, &'a NodeRecord>,
    incident: HashMap<&'a str, Vec<&'a EdgeRecord>>,
}

impl<'a> WikiGraph<'a> {
    fn new(document: &'a GraphDocument) -> Self {
        let nodes = document
            .nodes
            .iter()
            .map(|node| (node.id.as_str(), node))
            .collect();
        let mut incident = HashMap::<&str, Vec<&EdgeRecord>>::new();
        for edge in &document.links {
            incident.entry(edge.source.as_str()).or_default().push(edge);
            if !document.directed || edge.target != edge.source {
                incident.entry(edge.target.as_str()).or_default().push(edge);
            }
        }
        Self {
            directed: document.directed,
            nodes,
            incident,
        }
    }

    fn degree(&self, node: &str) -> usize {
        self.incident.get(node).map(Vec::len).unwrap_or_default()
    }

    fn neighbors(&self, node: &str) -> Vec<(&'a str, &'a EdgeRecord)> {
        self.incident
            .get(node)
            .into_iter()
            .flatten()
            .filter_map(|edge| {
                if edge.source == node {
                    Some((edge.target.as_str(), *edge))
                } else if !self.directed && edge.target == node {
                    Some((edge.source.as_str(), *edge))
                } else {
                    None
                }
            })
            .collect()
    }
}

#[allow(clippy::too_many_arguments)]
fn community_article(
    graph: &WikiGraph<'_>,
    community: usize,
    members: &[String],
    label: &str,
    labels: &BTreeMap<usize, String>,
    cohesion: Option<f64>,
    node_community: &HashMap<String, usize>,
    resolver: &HashMap<String, String>,
) -> String {
    let mut top_nodes = members.iter().collect::<Vec<_>>();
    top_nodes.sort_by_key(|node| std::cmp::Reverse(graph.degree(node)));
    top_nodes.truncate(25);
    let mut cross_counts = HashMap::<String, usize>::new();
    let mut cross_order = Vec::<String>::new();
    let mut confidence = HashMap::<String, usize>::new();
    for member in members {
        for (neighbor, edge) in graph.neighbors(member) {
            if let Some(other) = node_community
                .get(neighbor)
                .filter(|other| **other != community)
            {
                let label = labels
                    .get(other)
                    .cloned()
                    .unwrap_or_else(|| format!("Community {other}"));
                if !cross_counts.contains_key(&label) {
                    cross_order.push(label.clone());
                }
                *cross_counts.entry(label).or_default() += 1;
            }
            *confidence
                .entry(edge.string("confidence").if_empty("EXTRACTED"))
                .or_default() += 1;
        }
    }
    let mut cross = cross_order
        .into_iter()
        .map(|label| {
            let count = cross_counts.get(&label).copied().unwrap_or_default();
            (label, count)
        })
        .collect::<Vec<_>>();
    cross.sort_by_key(|(_, count)| std::cmp::Reverse(*count));
    let mut sources = members
        .iter()
        .filter_map(|member| graph.nodes.get(member.as_str()))
        .map(|node| node.string("source_file"))
        .filter(|source| !source.is_empty())
        .collect::<Vec<_>>();
    sources.sort();
    sources.dedup();

    let mut lines = vec![format!("# {label}"), String::new()];
    let mut metadata = vec![format!("{} nodes", members.len())];
    if let Some(cohesion) = cohesion {
        metadata.push(format!("cohesion {cohesion:.2}"));
    }
    lines.extend([format!("> {}", metadata.join(" · ")), String::new()]);
    lines.extend(["## Key Concepts".to_owned(), String::new()]);
    for member in top_nodes {
        let Some(node) = graph.nodes.get(member.as_str()) else {
            continue;
        };
        let source = node.string("source_file");
        let source = if source.is_empty() {
            String::new()
        } else {
            format!(" — `{source}`")
        };
        lines.push(format!(
            "- **{}** ({} connections){source}",
            node.label(),
            graph.degree(member)
        ));
    }
    let remaining = members.len().saturating_sub(25);
    if remaining > 0 {
        lines.push(format!(
            "- *... and {remaining} more nodes in this community*"
        ));
    }
    lines.push(String::new());
    lines.extend(["## Relationships".to_owned(), String::new()]);
    if cross.is_empty() {
        lines.push("- No strong cross-community connections detected".to_owned());
    } else {
        for (other, count) in cross.into_iter().take(12) {
            lines.push(format!(
                "- {} ({count} shared connections)",
                markdown_link(&other, resolver)
            ));
        }
    }
    lines.push(String::new());
    if !sources.is_empty() {
        lines.extend(["## Source Files".to_owned(), String::new()]);
        for source in sources.into_iter().take(20) {
            lines.push(format!("- `{source}`"));
        }
        lines.push(String::new());
    }
    lines.extend(["## Audit Trail".to_owned(), String::new()]);
    let total = confidence.values().sum::<usize>().max(1);
    for name in ["EXTRACTED", "INFERRED", "AMBIGUOUS"] {
        let count = confidence.get(name).copied().unwrap_or_default();
        let percentage = python_round_percent(count, total);
        lines.push(format!("- {name}: {count} ({percentage}%)"));
    }
    lines.extend([
        String::new(),
        "---".to_owned(),
        String::new(),
        format!(
            "*Part of the graphify knowledge wiki. See {} to navigate.*",
            markdown_link("index", resolver)
        ),
    ]);
    lines.join("\n")
}

fn god_node_article(
    graph: &WikiGraph<'_>,
    god: &GodNode,
    labels: &BTreeMap<usize, String>,
    node_community: &HashMap<String, usize>,
    resolver: &HashMap<String, String>,
) -> String {
    let Some(node) = graph.nodes.get(god.id.as_str()) else {
        return String::new();
    };
    let source = node.string("source_file");
    let mut lines = vec![
        format!("# {}", node.label()),
        String::new(),
        format!(
            "> God node · {} connections · `{source}`",
            graph.degree(&god.id)
        ),
        String::new(),
    ];
    if let Some(community) = node_community.get(&god.id) {
        let label = labels
            .get(community)
            .cloned()
            .unwrap_or_else(|| format!("Community {community}"));
        lines.extend([
            format!("**Community:** {}", markdown_link(&label, resolver)),
            String::new(),
        ]);
    }
    let mut neighbors = graph.neighbors(&god.id);
    neighbors.sort_by_key(|(neighbor, _)| std::cmp::Reverse(graph.degree(neighbor)));
    let mut by_relation = BTreeMap::<String, Vec<String>>::new();
    for (neighbor, edge) in neighbors {
        let Some(neighbor_node) = graph.nodes.get(neighbor) else {
            continue;
        };
        let confidence = edge.string("confidence");
        let suffix = if confidence.is_empty() {
            String::new()
        } else {
            format!(" `{confidence}`")
        };
        by_relation
            .entry(edge.string("relation").if_empty("related"))
            .or_default()
            .push(format!(
                "{}{suffix}",
                markdown_link(neighbor_node.label(), resolver)
            ));
    }
    lines.extend(["## Connections by Relation".to_owned(), String::new()]);
    for (relation, targets) in by_relation {
        lines.push(format!("### {relation}"));
        for target in targets.into_iter().take(20) {
            lines.push(format!("- {target}"));
        }
        lines.push(String::new());
    }
    lines.extend([
        "---".to_owned(),
        String::new(),
        format!(
            "*Part of the graphify knowledge wiki. See {} to navigate.*",
            markdown_link("index", resolver)
        ),
    ]);
    lines.join("\n")
}

fn index_markdown(
    communities: &Communities,
    labels: &BTreeMap<usize, String>,
    gods: &[GodNode],
    total_nodes: usize,
    total_edges: usize,
    resolver: &HashMap<String, String>,
) -> String {
    let mut output = format!(
        "# Knowledge Graph Index\n\n> Auto-generated by graphify. Start here — read community articles for context, then drill into god nodes for detail.\n\n**{total_nodes} nodes · {total_edges} edges · {} communities**\n\n---\n\n## Communities\n(sorted by size, largest first)\n\n",
        communities.len()
    );
    let mut ordered = communities.iter().collect::<Vec<_>>();
    ordered.sort_by_key(|(_, members)| std::cmp::Reverse(members.len()));
    for (community, members) in ordered {
        let label = &labels[community];
        let _ = writeln!(
            output,
            "- {} — {} nodes",
            markdown_link(label, resolver),
            members.len()
        );
    }
    output.push('\n');
    if !gods.is_empty() {
        output.push_str(
            "## God Nodes\n(most connected concepts — the load-bearing abstractions)\n\n",
        );
        for god in gods {
            let _ = writeln!(
                output,
                "- {} — {} connections",
                markdown_link(&god.label, resolver),
                god.degree
            );
        }
        output.push('\n');
    }
    output.push_str("---\n\n*Generated by [graphify](https://github.com/safishamsi/graphify)*");
    output
}

fn safe_filename(name: &str) -> String {
    let replaced = name
        .chars()
        .map(|character| match character {
            '/' => '-',
            ' ' => '_',
            ':' => '-',
            '<' | '>' | '"' | '\\' | '|' | '?' | '*' => '_',
            other => other,
        })
        .collect::<String>();
    let trimmed = replaced.trim_matches(['.', ' ']);
    let output = trimmed.chars().take(200).collect::<String>();
    if output.is_empty() {
        "unnamed".to_owned()
    } else {
        output
    }
}

fn unique_slug(base: &str, used: &mut HashSet<String>) -> String {
    let mut slug = base.to_owned();
    let mut suffix = 2;
    while used.contains(&slug.to_lowercase()) {
        slug = format!("{base}_{suffix}");
        suffix += 1;
    }
    used.insert(slug.to_lowercase());
    slug
}

fn markdown_link(label: &str, resolver: &HashMap<String, String>) -> String {
    let text = label.replace('[', "\\[").replace(']', "\\]");
    resolver.get(label).map_or(text.clone(), |slug| {
        format!("[{text}]({})", url_quote(&format!("{slug}.md")))
    })
}

fn url_quote(value: &str) -> String {
    let mut output = String::new();
    for byte in value.as_bytes() {
        if byte.is_ascii_alphanumeric() || matches!(*byte, b'-' | b'_' | b'.' | b'~' | b'/') {
            output.push(char::from(*byte));
        } else {
            let _ = write!(output, "%{byte:02X}");
        }
    }
    output
}

fn python_round_percent(count: usize, total: usize) -> usize {
    let value = count as f64 / total as f64 * 100.0;
    value.round_ties_even() as usize
}

fn wiki_io(path: impl Into<PathBuf>, source: std::io::Error) -> OutputError {
    OutputError::WikiIo {
        path: path.into(),
        source,
    }
}

trait EmptyFallback {
    fn if_empty(self, fallback: &str) -> String;
}

impl EmptyFallback for String {
    fn if_empty(self, fallback: &str) -> String {
        if self.is_empty() {
            fallback.to_owned()
        } else {
            self
        }
    }
}
