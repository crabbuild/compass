use std::collections::{BTreeMap, BTreeSet, HashMap};

use compass_graph::Communities;
use compass_model::{GraphDocument, NodeRecord};
use serde::Serialize;

use crate::{CallflowOptions, CallflowSection, OutputError, derive_callflow_sections};

pub const CALLFLOW_VIEWER_SCHEMA: &str = "compass.viewer.callflow/1";

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CallflowViewModel {
    pub schema: &'static str,
    pub title: String,
    pub sections: Vec<CallflowViewSection>,
    pub overview_links: Vec<CallflowViewLink>,
    pub report_highlights: Vec<String>,
    pub statistics: CallflowStatistics,
    pub provenance: CallflowProvenance,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CallflowViewSection {
    pub id: String,
    pub name: String,
    pub communities: Vec<String>,
    pub nodes: Vec<CallflowViewNode>,
    pub edges: Vec<CallflowViewEdge>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CallflowViewNode {
    pub id: String,
    pub label: String,
    pub kind: String,
    pub source_file: Option<String>,
}

#[derive(Clone, Debug, Serialize)]
pub struct CallflowViewEdge {
    pub source: String,
    pub target: String,
    pub relation: String,
    pub confidence: String,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CallflowViewLink {
    pub source_section: String,
    pub target_section: String,
    pub calls: usize,
}

#[derive(Clone, Debug, Serialize)]
pub struct CallflowStatistics {
    pub nodes: usize,
    pub edges: usize,
    pub communities: usize,
    pub hyperedges: usize,
    pub extracted: usize,
    pub inferred: usize,
    pub ambiguous: usize,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CallflowProvenance {
    pub project_name: String,
    pub built_at_commit: Option<String>,
    pub generated_at: Option<String>,
}

pub fn callflow_view_model(
    document: &GraphDocument,
    communities: &Communities,
    options: &CallflowOptions<'_>,
) -> Result<CallflowViewModel, OutputError> {
    if document.nodes.is_empty() {
        return Err(OutputError::EmptyCallflowGraph);
    }
    let mut sections = options.sections.map_or_else(
        || {
            derive_callflow_sections(
                document,
                communities,
                options.community_labels,
                options.language,
                options.max_sections,
            )
        },
        <[CallflowSection]>::to_vec,
    );
    if sections
        .first()
        .is_none_or(|section| section.id != "overview")
    {
        sections.insert(
            0,
            CallflowSection {
                id: "overview".to_owned(),
                name: "Architecture Overview".to_owned(),
                communities: Vec::new(),
            },
        );
    }
    if sections.len() <= 1 {
        return Err(OutputError::NoCallflowSections);
    }
    let node_community = communities
        .iter()
        .flat_map(|(community, members)| {
            members
                .iter()
                .map(move |member| (member.as_str(), community.to_string()))
        })
        .collect::<HashMap<_, _>>();
    let mut node_section = HashMap::<&str, String>::new();
    let mut view_sections = Vec::new();
    for section in sections {
        let selected = document
            .nodes
            .iter()
            .filter(|node| {
                let community = node_community.get(node.id.as_str()).cloned().or_else(|| {
                    node.attributes
                        .get("community")
                        .and_then(serde_json::Value::as_u64)
                        .map(|value| value.to_string())
                });
                section.id != "overview"
                    && community.is_some_and(|community| section.communities.contains(&community))
            })
            .collect::<Vec<_>>();
        for node in &selected {
            node_section.insert(&node.id, section.id.clone());
        }
        let ids = selected
            .iter()
            .map(|node| node.id.as_str())
            .collect::<BTreeSet<_>>();
        let nodes = selected.into_iter().map(view_node).collect::<Vec<_>>();
        let edges = document
            .links
            .iter()
            .filter(|edge| ids.contains(edge.source.as_str()) && ids.contains(edge.target.as_str()))
            .map(|edge| CallflowViewEdge {
                source: edge.source.clone(),
                target: edge.target.clone(),
                relation: edge.string("relation"),
                confidence: confidence(edge.string("confidence")),
            })
            .collect::<Vec<_>>();
        view_sections.push(CallflowViewSection {
            id: section.id,
            name: section.name,
            communities: section.communities,
            nodes,
            edges,
        });
    }
    let mut link_counts = BTreeMap::<(String, String), usize>::new();
    for edge in &document.links {
        let (Some(source), Some(target)) = (
            node_section.get(edge.source.as_str()),
            node_section.get(edge.target.as_str()),
        ) else {
            continue;
        };
        if source != target {
            *link_counts
                .entry((source.clone(), target.clone()))
                .or_default() += 1;
        }
    }
    let overview_links = link_counts
        .into_iter()
        .map(
            |((source_section, target_section), calls)| CallflowViewLink {
                source_section,
                target_section,
                calls,
            },
        )
        .collect();
    let confidence_counts = |expected: &str| {
        document
            .links
            .iter()
            .filter(|edge| confidence(edge.string("confidence")) == expected)
            .count()
    };
    let report_highlights = options
        .report
        .lines()
        .map(str::trim)
        .filter(|line| line.starts_with('#') || line.starts_with("- "))
        .map(|line| line.trim_start_matches(['#', '-', ' ']).to_owned())
        .filter(|line| !line.is_empty())
        .take(12)
        .collect();
    Ok(CallflowViewModel {
        schema: CALLFLOW_VIEWER_SCHEMA,
        title: format!("{} — Architecture Flow", options.project_name),
        sections: view_sections,
        overview_links,
        report_highlights,
        statistics: CallflowStatistics {
            nodes: document.nodes.len(),
            edges: document.links.len(),
            communities: communities.len(),
            hyperedges: document
                .graph
                .get("hyperedges")
                .and_then(serde_json::Value::as_array)
                .map_or(0, Vec::len),
            extracted: confidence_counts("extracted"),
            inferred: confidence_counts("inferred"),
            ambiguous: confidence_counts("ambiguous"),
        },
        provenance: CallflowProvenance {
            project_name: options.project_name.to_owned(),
            built_at_commit: options.built_at_commit.map(str::to_owned),
            generated_at: options.generated_at.map(str::to_owned),
        },
    })
}

fn view_node(node: &NodeRecord) -> CallflowViewNode {
    let source_file = node.string("source_file");
    CallflowViewNode {
        id: node.id.clone(),
        label: {
            let label = node.string("label");
            if label.is_empty() {
                node.id.clone()
            } else {
                label
            }
        },
        kind: {
            let kind = node.string("symbol_kind");
            if kind.is_empty() {
                node.string("file_type")
            } else {
                kind
            }
        },
        source_file: (!source_file.is_empty()).then_some(source_file),
    }
}

fn confidence(value: String) -> String {
    match value.to_ascii_lowercase().as_str() {
        "inferred" => "inferred",
        "ambiguous" => "ambiguous",
        _ => "extracted",
    }
    .to_owned()
}
