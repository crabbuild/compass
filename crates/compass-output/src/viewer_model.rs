use std::collections::BTreeSet;

use compass_graph::Communities;
use compass_model::GraphDocument;
use serde::Serialize;
use serde_json::{Map, Value};

use crate::html::{HtmlOptions, edge_value, node_values};

pub const GRAPH_VIEWER_SCHEMA: &str = "compass.viewer.graph/1";
const COLORS: [&str; 10] = [
    "#4E79A7", "#F28E2B", "#E15759", "#76B7B2", "#59A14F", "#EDC948", "#B07AA1", "#FF9DA7",
    "#9C755F", "#BAB0AC",
];

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GraphViewModel {
    pub schema: &'static str,
    pub title: String,
    pub stats: GraphViewStats,
    pub nodes: Vec<GraphViewNode>,
    pub edges: Vec<GraphViewEdge>,
    pub communities: Vec<GraphViewCommunity>,
    pub hyperedges: Vec<Value>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GraphViewStats {
    pub nodes: usize,
    pub edges: usize,
    pub communities: usize,
    pub aggregated: bool,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GraphViewNode {
    pub id: String,
    pub label: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub kind: Option<String>,
    pub community: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub community_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub degree: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source: Option<GraphViewSource>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub color: Option<GraphViewColor>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub learning_status: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub learning_stale: Option<bool>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GraphViewSource {
    pub file: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub start_line: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub end_line: Option<u64>,
}

#[derive(Clone, Debug, Serialize)]
pub struct GraphViewColor {
    pub background: String,
    pub border: String,
}

#[derive(Clone, Debug, Serialize)]
pub struct GraphViewEdge {
    pub id: String,
    pub source: String,
    pub target: String,
    pub relation: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub confidence: Option<String>,
}

#[derive(Clone, Debug, Serialize)]
pub struct GraphViewCommunity {
    pub id: usize,
    pub label: String,
    pub color: String,
    pub hidden: bool,
}

pub fn graph_view_model(
    document: &GraphDocument,
    communities: &Communities,
    title: impl Into<String>,
    options: &HtmlOptions<'_>,
    aggregated: bool,
) -> GraphViewModel {
    let nodes = node_values(document, communities, options)
        .into_iter()
        .filter_map(|value| {
            let object = value.as_object()?;
            let file = string(object, "source_file");
            let color = object.get("color").and_then(Value::as_object);
            Some(GraphViewNode {
                id: string(object, "id")?,
                label: string(object, "label").unwrap_or_default(),
                kind: non_empty(object, "symbol_kind"),
                community: object
                    .get("community")
                    .and_then(Value::as_u64)
                    .unwrap_or_default() as usize,
                community_name: non_empty(object, "community_name"),
                degree: object
                    .get("degree")
                    .and_then(Value::as_u64)
                    .map(|value| value as usize),
                source: file
                    .filter(|file| !file.is_empty())
                    .map(|file| GraphViewSource {
                        file,
                        start_line: object.get("line_start").and_then(Value::as_u64),
                        end_line: object.get("line_end").and_then(Value::as_u64),
                    }),
                color: color.and_then(|color| {
                    Some(GraphViewColor {
                        background: string(color, "background")?,
                        border: string(color, "border")?,
                    })
                }),
                learning_status: non_empty(object, "learning_status"),
                learning_stale: object.get("learning_stale").and_then(Value::as_bool),
            })
        })
        .collect::<Vec<_>>();

    let edges = document
        .links
        .iter()
        .enumerate()
        .filter_map(|(index, edge)| {
            let value = edge_value(edge);
            let object = value.as_object()?;
            let source = string(object, "from")?;
            let target = string(object, "to")?;
            Some(GraphViewEdge {
                id: format!("edge-{index}-{source}-{target}"),
                source,
                target,
                relation: string(object, "label").unwrap_or_default(),
                confidence: string(object, "confidence").map(|value| {
                    match value.to_ascii_lowercase().as_str() {
                        "extracted" => "extracted",
                        "ambiguous" => "ambiguous",
                        _ => "inferred",
                    }
                    .to_owned()
                }),
            })
        })
        .collect::<Vec<_>>();

    let ids = communities
        .keys()
        .copied()
        .chain(
            options
                .community_labels
                .into_iter()
                .flat_map(|labels| labels.keys().copied()),
        )
        .chain(nodes.iter().map(|node| node.community))
        .collect::<BTreeSet<_>>();
    let view_communities = ids
        .into_iter()
        .map(|id| GraphViewCommunity {
            id,
            label: options
                .community_labels
                .and_then(|labels| labels.get(&id))
                .cloned()
                .unwrap_or_else(|| format!("Community {id}")),
            color: COLORS[id % COLORS.len()].to_owned(),
            hidden: false,
        })
        .collect::<Vec<_>>();
    let hyperedges = document
        .graph
        .get("hyperedges")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();

    GraphViewModel {
        schema: GRAPH_VIEWER_SCHEMA,
        title: title.into(),
        stats: GraphViewStats {
            nodes: nodes.len(),
            edges: edges.len(),
            communities: view_communities.len(),
            aggregated,
        },
        nodes,
        edges,
        communities: view_communities,
        hyperedges,
    }
}

pub fn shared_viewer_html(model: &GraphViewModel) -> Result<String, serde_json::Error> {
    let model_json = serde_json::to_string(model)?
        .replace('<', "\\u003c")
        .replace('>', "\\u003e")
        .replace('&', "\\u0026");
    Ok(format!(
        r#"<!doctype html>
<html lang="en">
<head>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width,initial-scale=1">
<meta name="color-scheme" content="light dark">
<title>Compass — {title}</title>
<style>{css}</style>
</head>
<body>
<div id="compass-viewer-root"></div>
<script id="compass-viewer-model" type="application/json">{model_json}</script>
<script>{javascript}</script>
</body>
</html>
"#,
        title = escape_html(&model.title),
        css = include_str!("../assets/viewer/viewer.css"),
        javascript = include_str!("../assets/viewer/graph.js"),
    ))
}

fn string(object: &Map<String, Value>, key: &str) -> Option<String> {
    object.get(key).and_then(Value::as_str).map(str::to_owned)
}

fn non_empty(object: &Map<String, Value>, key: &str) -> Option<String> {
    string(object, key).filter(|value| !value.is_empty())
}

fn escape_html(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&#x27;")
}
