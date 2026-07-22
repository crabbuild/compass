use std::collections::{BTreeMap, HashMap};
use std::path::Path;

use compass_graph::{
    Communities, GodNode, SuggestedQuestion, SurpriseConnection, find_import_cycles,
};
use compass_model::{GraphDocument, NodeRecord};
use serde_json::Value;

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct DetectionSummary {
    pub total_files: usize,
    pub total_words: usize,
    pub warning: Option<String>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct TokenCost {
    pub input: u64,
    pub output: u64,
}

#[derive(Clone, Debug)]
pub struct ReportOptions<'a> {
    pub root: &'a str,
    pub min_community_size: usize,
    pub built_at_commit: Option<&'a str>,
    pub obsidian: bool,
    pub today: Option<&'a str>,
}

impl<'a> ReportOptions<'a> {
    #[must_use]
    pub fn new(root: &'a str) -> Self {
        Self {
            root,
            min_community_size: 3,
            built_at_commit: None,
            obsidian: false,
            today: None,
        }
    }
}

#[allow(clippy::too_many_arguments)]
#[must_use]
pub fn generate_report(
    document: &GraphDocument,
    communities: &Communities,
    cohesion_scores: &BTreeMap<usize, f64>,
    community_labels: &BTreeMap<usize, String>,
    god_node_list: &[GodNode],
    surprise_list: &[SurpriseConnection],
    detection: &DetectionSummary,
    token_cost: TokenCost,
    suggested_questions: Option<&[SuggestedQuestion]>,
    learning: Option<&Value>,
    options: &ReportOptions<'_>,
) -> String {
    let today = options.today.map_or_else(current_date, str::to_owned);
    let graph = ReportGraph::new(document);
    let confidences = graph
        .edges
        .iter()
        .map(|edge| edge_attribute(edge, "confidence").unwrap_or("EXTRACTED"))
        .collect::<Vec<_>>();
    let total = confidences.len().max(1);
    let extracted_percent = percentage(
        confidences
            .iter()
            .filter(|value| **value == "EXTRACTED")
            .count(),
        total,
    );
    let inferred_percent = percentage(
        confidences
            .iter()
            .filter(|value| **value == "INFERRED")
            .count(),
        total,
    );
    let ambiguous_percent = percentage(
        confidences
            .iter()
            .filter(|value| **value == "AMBIGUOUS")
            .count(),
        total,
    );
    let inferred = graph
        .edges
        .iter()
        .filter(|edge| edge_attribute(edge, "confidence") == Some("INFERRED"))
        .collect::<Vec<_>>();
    let inferred_average = if inferred.is_empty() {
        None
    } else {
        Some(round_two(
            inferred
                .iter()
                .map(|edge| {
                    edge.attributes
                        .get("confidence_score")
                        .and_then(Value::as_f64)
                        .unwrap_or(0.5)
                })
                .sum::<f64>()
                / inferred.len() as f64,
        ))
    };

    let mut lines = vec![
        format!("# Graph Report - {}  ({today})", options.root),
        String::new(),
        "## Corpus Check".to_owned(),
    ];
    if let Some(warning) = &detection.warning {
        lines.push(format!("- {warning}"));
    } else {
        lines.push(format!(
            "- {} files · ~{} words",
            detection.total_files,
            grouped(detection.total_words as u64)
        ));
        lines.push("- Verdict: corpus is large enough that graph structure adds value.".to_owned());
    }

    let non_empty = communities
        .iter()
        .filter(|(_, members)| members.iter().any(|member| !graph.is_file_node_id(member)))
        .collect::<Vec<_>>();
    let thin_count = communities
        .values()
        .filter(|members| {
            let count = members
                .iter()
                .filter(|member| !graph.is_file_node_id(member))
                .count();
            count > 0 && count < options.min_community_size
        })
        .count();
    let shown_count = communities.len() - thin_count;
    let thin_suffix = if thin_count == 0 {
        String::new()
    } else {
        format!(" ({shown_count} shown, {thin_count} thin omitted)")
    };
    let inferred_suffix = inferred_average.map_or_else(String::new, |average| {
        format!(
            " · INFERRED: {} edges (avg confidence: {average})",
            inferred.len()
        )
    });
    lines.extend([
        String::new(),
        "## Summary".to_owned(),
        format!(
            "- {} nodes · {} edges · {} communities{thin_suffix}",
            graph.nodes.len(),
            graph.edges.len(),
            communities.len()
        ),
        format!(
            "- Extraction: {extracted_percent}% EXTRACTED · {inferred_percent}% INFERRED · {ambiguous_percent}% AMBIGUOUS{inferred_suffix}"
        ),
        format!(
            "- Token cost: {} input · {} output",
            grouped(token_cost.input),
            grouped(token_cost.output)
        ),
    ]);
    if let Some(commit) = options.built_at_commit.filter(|commit| !commit.is_empty()) {
        lines.extend([
            String::new(),
            "## Graph Freshness".to_owned(),
            format!("- Built from commit: `{}`", prefix_chars(commit, 8)),
            "- Run `git rev-parse HEAD` and compare to check if the graph is stale.".to_owned(),
            "- Run `graphify update .` after code changes (no API cost).".to_owned(),
        ]);
    }
    if !non_empty.is_empty() {
        lines.extend([String::new(), "## Community Hubs (Navigation)".to_owned()]);
        for (community, _) in non_empty {
            let label = community_labels
                .get(community)
                .cloned()
                .unwrap_or_else(|| format!("Community {community}"));
            if options.obsidian {
                lines.push(format!(
                    "- [[_COMMUNITY_{}|{label}]]",
                    safe_community_name(&label)
                ));
            } else {
                lines.push(format!("- {label}"));
            }
        }
    }

    lines.extend([
        String::new(),
        "## God Nodes (most connected - your core abstractions)".to_owned(),
    ]);
    for (index, node) in god_node_list.iter().enumerate() {
        lines.push(format!(
            "{}. `{}` - {} edges",
            index + 1,
            node.label,
            node.degree
        ));
    }
    lines.extend([
        String::new(),
        "## Surprising Connections (you probably didn't know these)".to_owned(),
    ]);
    if surprise_list.is_empty() {
        lines
            .push("- None detected - all connections are within the same source files.".to_owned());
    } else {
        for surprise in surprise_list {
            let semantic = if surprise.relation == "semantically_similar_to" {
                " [semantically similar]"
            } else {
                ""
            };
            lines.push(format!(
                "- `{}` --{}--> `{}`  [{}]{semantic}",
                surprise.source, surprise.relation, surprise.target, surprise.confidence
            ));
            let note = surprise
                .note
                .as_ref()
                .map_or_else(String::new, |note| format!("  _{note}_"));
            lines.push(format!(
                "  {} → {}{note}",
                surprise.source_files[0], surprise.source_files[1]
            ));
        }
    }

    let has_code = graph
        .nodes
        .iter()
        .any(|node| attribute(node, "file_type") == Some("code"))
        || graph.edges.iter().any(|edge| {
            matches!(
                edge_attribute(edge, "relation"),
                Some("imports" | "imports_from")
            )
        });
    if has_code {
        lines.extend([String::new(), "## Import Cycles".to_owned()]);
        let cycles = find_import_cycles(document, 5, 20);
        if cycles.is_empty() {
            lines.push("- None detected.".to_owned());
        } else {
            for cycle in cycles {
                if cycle.cycle.is_empty() {
                    continue;
                }
                let mut path = cycle.cycle.clone();
                path.push(cycle.cycle[0].clone());
                lines.push(format!(
                    "- {}-file cycle: `{}`",
                    cycle.length,
                    path.join(" -> ")
                ));
            }
        }
    }

    if let Some(hyperedges) = document
        .graph
        .get("hyperedges")
        .and_then(Value::as_array)
        .filter(|values| !values.is_empty())
    {
        lines.extend([
            String::new(),
            "## Hyperedges (group relationships)".to_owned(),
        ]);
        for hyperedge in hyperedges {
            let id = hyperedge
                .get("label")
                .or_else(|| hyperedge.get("id"))
                .and_then(Value::as_str)
                .unwrap_or_default();
            let members = hyperedge
                .get("nodes")
                .and_then(Value::as_array)
                .map(|values| {
                    values
                        .iter()
                        .filter_map(Value::as_str)
                        .collect::<Vec<_>>()
                        .join(", ")
                })
                .unwrap_or_default();
            let confidence = hyperedge
                .get("confidence")
                .and_then(Value::as_str)
                .unwrap_or("INFERRED");
            let confidence_tag = hyperedge
                .get("confidence_score")
                .and_then(Value::as_f64)
                .map_or_else(
                    || confidence.to_owned(),
                    |score| format!("{confidence} {score:.2}"),
                );
            lines.push(format!("- **{id}** — {members} [{confidence_tag}]"));
        }
    }

    lines.extend([
        String::new(),
        format!(
            "## Communities ({} total, {thin_count} thin omitted)",
            communities.len()
        ),
    ]);
    for (community, members) in communities {
        let real_nodes = members
            .iter()
            .filter(|member| !graph.is_file_node_id(member))
            .collect::<Vec<_>>();
        if real_nodes.len() < options.min_community_size {
            continue;
        }
        let label = community_labels
            .get(community)
            .cloned()
            .unwrap_or_else(|| format!("Community {community}"));
        let score = cohesion_scores.get(community).copied().unwrap_or_default();
        let display = real_nodes
            .iter()
            .take(8)
            .map(|member| graph.label(member))
            .collect::<Vec<_>>();
        let suffix = if real_nodes.len() > 8 {
            format!(" (+{} more)", real_nodes.len() - 8)
        } else {
            String::new()
        };
        lines.extend([
            String::new(),
            format!("### Community {community} - \"{label}\""),
            format!("Cohesion: {score:.2}"),
            format!(
                "Nodes ({}): {}{suffix}",
                real_nodes.len(),
                display.join(", ")
            ),
        ]);
    }

    let ambiguous = graph
        .edges
        .iter()
        .filter(|edge| edge_attribute(edge, "confidence") == Some("AMBIGUOUS"))
        .collect::<Vec<_>>();
    if !ambiguous.is_empty() {
        lines.extend([
            String::new(),
            "## Ambiguous Edges - Review These".to_owned(),
        ]);
        for edge in ambiguous {
            lines.push(format!(
                "- `{}` → `{}`  [AMBIGUOUS]",
                graph.label(&edge.source),
                graph.label(&edge.target)
            ));
            lines.push(format!(
                "  {} · relation: {}",
                edge_attribute(edge, "source_file").unwrap_or_default(),
                edge_attribute(edge, "relation").unwrap_or("unknown")
            ));
        }
    }

    let isolated = graph
        .nodes
        .iter()
        .filter(|node| {
            graph.degree(&node.id) <= 1
                && !graph.is_file_node_id(&node.id)
                && !is_concept_node(node)
                && attribute(node, "file_type") != Some("rationale")
        })
        .collect::<Vec<_>>();
    let thin_communities = communities
        .values()
        .filter(|members| {
            let count = members
                .iter()
                .filter(|member| !graph.is_file_node_id(member))
                .count();
            count > 0 && count < 3
        })
        .count();
    if !isolated.is_empty() || thin_communities > 0 || ambiguous_percent > 20 {
        lines.extend([String::new(), "## Knowledge Gaps".to_owned()]);
        if !isolated.is_empty() {
            let labels = isolated
                .iter()
                .take(5)
                .map(|node| format!("`{}`", node.label()))
                .collect::<Vec<_>>()
                .join(", ");
            let suffix = if isolated.len() > 5 {
                format!(" (+{} more)", isolated.len() - 5)
            } else {
                String::new()
            };
            lines.push(format!(
                "- **{} isolated node(s):** {labels}{suffix}",
                isolated.len()
            ));
            lines.push(
                "  These have ≤1 connection - possible missing edges or undocumented components."
                    .to_owned(),
            );
        }
        if thin_communities > 0 {
            lines.push(format!("- **{thin_communities} thin communities (<{} nodes) omitted from report** — run `graphify query` to explore isolated nodes.", options.min_community_size));
        }
        if ambiguous_percent > 20 {
            lines.push(format!("- **High ambiguity: {ambiguous_percent}% of edges are AMBIGUOUS.** Review the Ambiguous Edges section above."));
        }
    }
    append_learning(&mut lines, learning);
    if let Some(questions) = suggested_questions.filter(|questions| !questions.is_empty()) {
        lines.extend([String::new(), "## Suggested Questions".to_owned()]);
        if questions.len() == 1 && questions[0].kind == "no_signal" {
            lines.push(format!("_{}_", questions[0].why));
        } else {
            lines.extend([
                "_Questions this graph is uniquely positioned to answer:_".to_owned(),
                String::new(),
            ]);
            for question in questions {
                if let Some(text) = &question.question {
                    lines.push(format!("- **{text}**"));
                    lines.push(format!("  _{}_", question.why));
                }
            }
        }
    }
    lines.join("\n")
}

fn append_learning(lines: &mut Vec<String>, learning: Option<&Value>) {
    let Some(learning) = learning else {
        return;
    };
    let mut preferred = learning
        .get("overlay")
        .and_then(Value::as_object)
        .map(|overlay| {
            overlay
                .iter()
                .filter(|(_, entry)| {
                    entry.get("status").and_then(Value::as_str) == Some("preferred")
                })
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    preferred.sort_by(|(left_id, left), (right_id, right)| {
        value_i64(right, "uses")
            .cmp(&value_i64(left, "uses"))
            .then_with(|| {
                value_f64(right, "score")
                    .partial_cmp(&value_f64(left, "score"))
                    .unwrap_or(std::cmp::Ordering::Equal)
            })
            .then_with(|| left_id.cmp(right_id))
    });
    let dead_ends = learning
        .get("dead_ends")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    if preferred.is_empty() && dead_ends.is_empty() {
        return;
    }
    lines.extend([String::new(), "## Work-memory lessons".to_owned()]);
    if !preferred.is_empty() {
        lines.extend([
            String::new(),
            "**Preferred sources** — corroborated by past sessions; start here.".to_owned(),
        ]);
        for (id, entry) in preferred.into_iter().take(10) {
            let label = entry.get("label").and_then(Value::as_str).unwrap_or(id);
            let stale = if entry.get("stale").and_then(Value::as_bool) == Some(true) {
                " _(code changed — re-verify)_"
            } else {
                ""
            };
            lines.push(format!(
                "- `{label}` ({}× useful, score={}){stale}",
                value_i64(entry, "uses"),
                number_text(entry.get("score"))
            ));
        }
    }
    if !dead_ends.is_empty() {
        lines.extend([
            String::new(),
            "**Known dead ends** — questions that led nowhere; don't re-derive.".to_owned(),
        ]);
        for dead_end in dead_ends {
            let question = dead_end
                .get("question")
                .and_then(Value::as_str)
                .unwrap_or_default();
            let nodes = dead_end
                .get("nodes")
                .and_then(Value::as_array)
                .map(|nodes| {
                    nodes
                        .iter()
                        .filter_map(Value::as_str)
                        .map(|node| format!("`{node}`"))
                        .collect::<Vec<_>>()
                        .join(", ")
                })
                .unwrap_or_default();
            lines.push(if nodes.is_empty() {
                format!("- \"{question}\"")
            } else {
                format!("- \"{question}\" -> {nodes}")
            });
        }
    }
}

struct ReportGraph<'a> {
    nodes: &'a [NodeRecord],
    edges: &'a [compass_model::EdgeRecord],
    positions: HashMap<&'a str, &'a NodeRecord>,
    degrees: HashMap<&'a str, usize>,
}
impl<'a> ReportGraph<'a> {
    fn new(document: &'a GraphDocument) -> Self {
        let positions = document
            .nodes
            .iter()
            .map(|node| (node.id.as_str(), node))
            .collect();
        let mut degrees = HashMap::new();
        for edge in &document.links {
            *degrees.entry(edge.source.as_str()).or_default() += 1;
            *degrees.entry(edge.target.as_str()).or_default() += 1;
        }
        Self {
            nodes: &document.nodes,
            edges: &document.links,
            positions,
            degrees,
        }
    }
    fn degree(&self, id: &str) -> usize {
        self.degrees.get(id).copied().unwrap_or_default()
    }
    fn label(&self, id: &str) -> String {
        self.positions
            .get(id)
            .map_or_else(|| id.to_owned(), |node| node.label().to_owned())
    }
    fn is_file_node_id(&self, id: &str) -> bool {
        let Some(node) = self.positions.get(id) else {
            return false;
        };
        let label = node
            .attributes
            .get("label")
            .and_then(Value::as_str)
            .unwrap_or_default();
        if label.is_empty() {
            return false;
        }
        let source = attribute(node, "source_file").unwrap_or_default();
        (!source.is_empty()
            && Path::new(source).file_name().and_then(|name| name.to_str()) == Some(label))
            || (label.starts_with('.') && label.ends_with("()"))
            || (label.ends_with("()") && self.degree(id) <= 1)
    }
}

fn current_date() -> String {
    time::OffsetDateTime::now_local()
        .unwrap_or_else(|_| time::OffsetDateTime::now_utc())
        .date()
        .to_string()
}
fn percentage(count: usize, total: usize) -> i64 {
    (count as f64 / total as f64 * 100.0).round() as i64
}
fn round_two(value: f64) -> f64 {
    (value * 100.0).round() / 100.0
}
fn grouped(value: u64) -> String {
    let digits = value.to_string();
    let mut output = String::new();
    for (index, character) in digits.chars().enumerate() {
        if index > 0 && (digits.len() - index).is_multiple_of(3) {
            output.push(',');
        }
        output.push(character);
    }
    output
}
fn prefix_chars(value: &str, count: usize) -> String {
    value.chars().take(count).collect()
}
fn attribute<'a>(node: &'a NodeRecord, key: &str) -> Option<&'a str> {
    node.attributes.get(key).and_then(Value::as_str)
}
fn edge_attribute<'a>(edge: &'a compass_model::EdgeRecord, key: &str) -> Option<&'a str> {
    edge.attributes.get(key).and_then(Value::as_str)
}
fn is_concept_node(node: &NodeRecord) -> bool {
    let source = attribute(node, "source_file").unwrap_or_default();
    source.is_empty() || !source.rsplit('/').next().unwrap_or_default().contains('.')
}
fn safe_community_name(label: &str) -> String {
    let single_line = label.replace("\r\n", " ").replace(['\r', '\n'], " ");
    let mut output = single_line
        .chars()
        .filter(|character| {
            !matches!(
                character,
                '\\' | '/' | '*' | '?' | ':' | '"' | '<' | '>' | '|' | '#' | '^' | '[' | ']'
            )
        })
        .collect::<String>()
        .trim()
        .to_owned();
    for extension in [".markdown", ".mdx", ".md"] {
        if output.to_lowercase().ends_with(extension) {
            output.truncate(output.len() - extension.len());
            break;
        }
    }
    if output.is_empty() {
        "unnamed".to_owned()
    } else {
        output
    }
}
fn value_i64(value: &Value, key: &str) -> i64 {
    value.get(key).and_then(Value::as_i64).unwrap_or_default()
}
fn value_f64(value: &Value, key: &str) -> f64 {
    value.get(key).and_then(Value::as_f64).unwrap_or_default()
}
fn number_text(value: Option<&Value>) -> String {
    value.map_or_else(
        || "0".to_owned(),
        |value| match value {
            Value::Number(number) => number.to_string(),
            _ => "0".to_owned(),
        },
    )
}
