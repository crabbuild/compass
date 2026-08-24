use std::collections::{BTreeMap, BTreeSet};

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
    #[serde(skip_serializing_if = "Option::is_none")]
    pub effective_graph: Option<Box<EffectiveGraphViewContext>>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EffectiveGraphViewContext {
    pub effective_identity: compass_agent_graph::Digest,
    pub base_generation: compass_agent_graph::BaseGenerationId,
    pub overlay_revision: compass_agent_graph::OverlayRevisionId,
    pub composition_profile: compass_agent_graph::CompositionProfile,
    pub retractions: compass_agent_graph::EffectiveRetractions,
    pub omissions: compass_agent_graph::CompositionOmissions,
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
    pub language: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub signature: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub size: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub member_count: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub detail_available: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source: Option<GraphViewSource>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub color: Option<GraphViewColor>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub learning_status: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub learning_stale: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub depth: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub root: Option<bool>,
    #[serde(rename = "agentAssertion", skip_serializing_if = "Option::is_none")]
    pub agent_assertion: Option<compass_agent_graph::AssertionId>,
    #[serde(rename = "agentSummary", skip_serializing_if = "Option::is_none")]
    pub agent_summary: Option<String>,
    #[serde(rename = "groundingStatus", skip_serializing_if = "Option::is_none")]
    pub grounding_status: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub challenged: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub challenge: Option<compass_agent_graph::EffectiveChallenge>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub document: Option<GraphViewDocument>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GraphViewDocument {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub role: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub kind: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub format: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ordinal: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub complete: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub visual_coverage: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ocr_mode: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub origin: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub locator: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ocr_profile: Option<Value>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GraphViewSource {
    pub file: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub start_line: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub end_line: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub start_byte: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub end_byte: Option<u64>,
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
    pub weight: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub confidence: Option<String>,
    #[serde(rename = "relationshipSite", skip_serializing_if = "Option::is_none")]
    pub relationship_site: Option<GraphViewSource>,
    #[serde(rename = "agentAssertion", skip_serializing_if = "Option::is_none")]
    pub agent_assertion: Option<compass_agent_graph::AssertionId>,
    #[serde(rename = "agentSummary", skip_serializing_if = "Option::is_none")]
    pub agent_summary: Option<String>,
    #[serde(rename = "groundingStatus", skip_serializing_if = "Option::is_none")]
    pub grounding_status: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub challenged: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub challenge: Option<compass_agent_graph::EffectiveChallenge>,
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
                language: non_empty(object, "language"),
                signature: non_empty(object, "signature"),
                size: object.get("size").and_then(Value::as_f64),
                member_count: object
                    .get("member_count")
                    .and_then(Value::as_u64)
                    .map(|value| value as usize),
                detail_available: None,
                source: file
                    .filter(|file| !file.is_empty())
                    .map(|file| GraphViewSource {
                        file,
                        start_line: object.get("line_start").and_then(Value::as_u64),
                        end_line: object.get("line_end").and_then(Value::as_u64),
                        start_byte: object.get("start_byte").and_then(Value::as_u64),
                        end_byte: object.get("end_byte").and_then(Value::as_u64),
                    }),
                color: color.and_then(|color| {
                    Some(GraphViewColor {
                        background: string(color, "background")?,
                        border: string(color, "border")?,
                    })
                }),
                learning_status: non_empty(object, "learning_status"),
                learning_stale: object.get("learning_stale").and_then(Value::as_bool),
                depth: None,
                root: None,
                agent_assertion: None,
                agent_summary: None,
                grounding_status: None,
                challenged: None,
                challenge: None,
                document: object.get("document").and_then(document_from_value),
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
            let relationship_site = graph_view_edge_source(edge);
            Some(GraphViewEdge {
                id: format!("edge-{index}-{source}-{target}"),
                source,
                target,
                relation: string(object, "label").unwrap_or_default(),
                weight: object
                    .get("weight")
                    .and_then(Value::as_u64)
                    .filter(|value| *value > 0)
                    .map(|value| value as usize),
                confidence: string(object, "confidence").map(|value| {
                    match value.to_ascii_lowercase().as_str() {
                        "extracted" => "extracted",
                        "ambiguous" => "ambiguous",
                        "aggregated" => "aggregated",
                        _ => "inferred",
                    }
                    .to_owned()
                }),
                relationship_site,
                agent_assertion: None,
                agent_summary: None,
                grounding_status: None,
                challenged: None,
                challenge: None,
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
        effective_graph: None,
    }
}

pub fn effective_graph_view_model(
    effective: &compass_agent_graph::EffectiveGraph,
    title: impl Into<String>,
    options: &HtmlOptions<'_>,
) -> Result<GraphViewModel, compass_model::GraphError> {
    let document = effective.graph.to_legacy_document()?;
    // Agent assertions can change topology. Recompute the communities from the
    // exact Effective Graph and discard labels/counts derived from the base so
    // a viewer can never present a mixed-revision topology.
    let communities = compass_graph::cluster(&document, compass_graph::ClusterOptions::default());
    let effective_options = HtmlOptions {
        community_labels: None,
        member_counts: None,
        node_limit: options.node_limit,
        learning_overlay: options.learning_overlay,
    };
    let mut model = graph_view_model(&document, &communities, title, &effective_options, false);
    let facts = effective
        .agent_facts
        .iter()
        .map(|fact| (fact.projected_id.as_str(), fact))
        .collect::<BTreeMap<_, _>>();
    let challenged = effective
        .challenges
        .iter()
        .map(|challenge| (challenge.target_id.as_str(), challenge))
        .collect::<BTreeMap<_, _>>();
    for node in &mut model.nodes {
        if let Some(fact) = facts.get(node.id.as_str()) {
            node.agent_assertion = Some(fact.assertion.clone());
            node.agent_summary = Some(fact.summary.clone());
            node.grounding_status = Some("GROUNDED".to_owned());
            node.color = Some(GraphViewColor {
                background: "#153d3a".to_owned(),
                border: "#5eead4".to_owned(),
            });
        }
        node.challenge = challenged
            .get(node.id.as_str())
            .map(|value| (*value).clone());
        node.challenged = node.challenge.as_ref().map(|_| true);
    }
    for (edge, record) in model.edges.iter_mut().zip(&document.links) {
        let edge_id = record.string("id");
        if let Some(fact) = facts.get(edge_id.as_str()) {
            edge.agent_assertion = Some(fact.assertion.clone());
            edge.agent_summary = Some(fact.summary.clone());
            edge.grounding_status = Some("GROUNDED".to_owned());
        }
        edge.challenge = challenged
            .get(edge_id.as_str())
            .map(|value| (*value).clone());
        edge.challenged = edge.challenge.as_ref().map(|_| true);
    }
    model.effective_graph = Some(Box::new(EffectiveGraphViewContext {
        effective_identity: effective.effective_identity.clone(),
        base_generation: effective.base_generation.clone(),
        overlay_revision: effective.overlay_revision.clone(),
        composition_profile: effective.composition_profile,
        retractions: effective.retractions.clone(),
        omissions: effective.omissions.clone(),
    }));
    Ok(model)
}

fn graph_view_edge_source(edge: &compass_model::EdgeRecord) -> Option<GraphViewSource> {
    let relationship_site = edge
        .attributes
        .get("relationshipSite")
        .and_then(Value::as_object);
    let file = relationship_site
        .and_then(|site| non_empty(site, "file"))
        .or_else(|| non_empty(&edge.attributes, "source_file"))?;
    let (location_start, location_end) = source_line_range(&edge.string("source_location"));
    Some(GraphViewSource {
        file,
        start_line: relationship_site
            .and_then(|site| unsigned(site, "startLine"))
            .or_else(|| unsigned(&edge.attributes, "line_start"))
            .or(location_start),
        end_line: relationship_site
            .and_then(|site| unsigned(site, "endLine"))
            .or_else(|| unsigned(&edge.attributes, "line_end"))
            .or(location_end)
            .or(location_start),
        start_byte: relationship_site
            .and_then(|site| unsigned(site, "startByte"))
            .or_else(|| unsigned(&edge.attributes, "start_byte")),
        end_byte: relationship_site
            .and_then(|site| unsigned(site, "endByte"))
            .or_else(|| unsigned(&edge.attributes, "end_byte")),
    })
}

fn source_line_range(location: &str) -> (Option<u64>, Option<u64>) {
    let Some(location) = location.strip_prefix('L') else {
        return (None, None);
    };
    let (start, end) = location
        .split_once('-')
        .map_or((location, None), |(start, end)| (start, Some(end)));
    let line = |value: &str| {
        value
            .strip_prefix('L')
            .unwrap_or(value)
            .split(':')
            .next()
            .and_then(|line| line.parse().ok())
    };
    (line(start), end.and_then(line))
}

fn unsigned(object: &Map<String, Value>, key: &str) -> Option<u64> {
    object.get(key).and_then(Value::as_u64)
}

pub fn shared_viewer_html(model: &GraphViewModel) -> Result<String, serde_json::Error> {
    shared_viewer_html_with_communities(model, &BTreeMap::new())
}

/// Render the shared offline graph workbench with optional community details.
///
/// Each detail remains inert JSON text until selected in the browser, keeping
/// initial model validation and React rendering bounded to the overview.
pub fn shared_viewer_html_with_communities(
    model: &GraphViewModel,
    community_details: &BTreeMap<usize, GraphViewModel>,
) -> Result<String, serde_json::Error> {
    let model_json = serde_json::to_string(model)?
        .replace('<', "\\u003c")
        .replace('>', "\\u003e")
        .replace('&', "\\u0026");
    let community_json = community_details
        .iter()
        .map(|(community, detail)| {
            serde_json::to_string(detail).map(|json| {
                format!(
                    r#"<script type="application/json" data-compass-community="{community}">{}</script>"#,
                    json.replace('<', "\\u003c")
                        .replace('>', "\\u003e")
                        .replace('&', "\\u0026")
                )
            })
        })
        .collect::<Result<String, _>>()?;
    Ok(format!(
        r#"<!doctype html>
<html lang="en">
<head>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width,initial-scale=1">
<meta name="color-scheme" content="light dark">
<meta http-equiv="Content-Security-Policy" content="default-src 'none'; img-src data:; style-src 'unsafe-inline'; script-src 'unsafe-inline';">
<title>Compass — {title}</title>
<style>{css}</style>
</head>
<body>
<div id="compass-viewer-root"></div>
<script id="compass-viewer-model" type="application/json">{model_json}</script>
{community_json}
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

fn document_from_value(value: &Value) -> Option<GraphViewDocument> {
    let object = value.as_object()?;
    Some(GraphViewDocument {
        role: string(object, "role"),
        kind: string(object, "kind"),
        format: string(object, "format"),
        text: string(object, "text"),
        ordinal: object.get("ordinal").and_then(Value::as_u64),
        complete: object.get("complete").and_then(Value::as_bool),
        visual_coverage: string(object, "visualCoverage"),
        ocr_mode: string(object, "ocrMode"),
        origin: object.get("origin").cloned(),
        locator: object.get("locator").cloned(),
        ocr_profile: object.get("ocrProfile").cloned(),
    })
}

fn escape_html(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&#x27;")
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use compass_graph::Communities;
    use compass_model::GraphDocument;
    use serde_json::json;

    use super::*;

    #[test]
    fn graph_model_preserves_sanitized_presentation_metadata()
    -> Result<(), Box<dyn std::error::Error>> {
        let document: GraphDocument = serde_json::from_value(json!({
            "nodes": [{
                "id": "run",
                "label": "run",
                "source_file": "src/main.rs",
                "line_start": 4,
                "line_end": 8,
                "symbol_kind": "function",
                "language": "rust",
                "signature": "fn run(value: usize)"
            }],
            "links": []
        }))?;
        let communities: Communities = BTreeMap::from([(0, vec!["run".into()])]);
        let member_counts = BTreeMap::from([(0, 7)]);
        let learning_overlay = BTreeMap::from([(
            "run".to_owned(),
            json!({"status": "preferred", "stale": false}),
        )]);
        let model = graph_view_model(
            &document,
            &communities,
            "Fixture",
            &HtmlOptions {
                member_counts: Some(&member_counts),
                learning_overlay: Some(&learning_overlay),
                ..HtmlOptions::default()
            },
            true,
        );
        let node = &model.nodes[0];
        assert_eq!(node.language.as_deref(), Some("rust"));
        assert_eq!(node.signature.as_deref(), Some("fn run(value: usize)"));
        assert!(node.size.is_some_and(|size| size > 0.0));
        assert_eq!(node.member_count, Some(7));
        assert_eq!(node.learning_status.as_deref(), Some("preferred"));
        assert_eq!(node.learning_stale, Some(false));
        assert_eq!(
            node.source.as_ref().and_then(|source| source.start_line),
            Some(4)
        );
        assert_eq!(
            node.source.as_ref().and_then(|source| source.end_line),
            Some(8)
        );
        Ok(())
    }

    #[test]
    fn graph_model_exposes_document_ocr_provenance_for_the_viewer()
    -> Result<(), Box<dyn std::error::Error>> {
        let document: GraphDocument = serde_json::from_value(json!({
            "nodes": [{
                "id": "scan",
                "label": "scan.pdf",
                "file_type": "document",
                "document_kind": "document",
                "document_format": "pdf",
                "document_visual_coverage": "partial",
                "document_ocr_mode": "auto",
                "document_complete": false,
                "document_ocr_profile": {
                    "engine": "OAR-OCR",
                    "engine_version": "0.9.2",
                    "profile": "pp-ocrv6-small"
                },
                "source_file": "docs/scan.pdf"
            }, {
                "id": "region",
                "label": "Invoice total",
                "file_type": "document",
                "document_kind": "paragraph",
                "document_text": "Invoice total",
                "document_origin": {
                    "kind": "ocr",
                    "profile": {
                        "engine": "OAR-OCR",
                        "engine_version": "0.9.2",
                        "profile": "pp-ocrv6-small"
                    },
                    "confidence_bps": 9234
                },
                "document_locator": {
                    "kind": "ocr",
                    "owner": {"kind": "pdf", "page": 4, "item": 1},
                    "candidate_id": "page-4",
                    "width": 1000,
                    "height": 800,
                    "polygon": [{"x": 10, "y": 12}],
                    "occurrence": 0
                },
                "block_index": 1,
                "source_file": "docs/scan.pdf"
            }],
            "links": []
        }))?;
        let communities: Communities =
            BTreeMap::from([(0, vec!["scan".to_owned(), "region".to_owned()])]);
        let model = graph_view_model(
            &document,
            &communities,
            "OCR",
            &HtmlOptions::default(),
            false,
        );
        let json = serde_json::to_value(&model.nodes)?;
        assert_eq!(json[0]["document"]["role"], "root");
        assert_eq!(json[0]["document"]["ocrMode"], "auto");
        assert_eq!(json[0]["document"]["visualCoverage"], "partial");
        assert_eq!(json[1]["document"]["origin"]["kind"], "ocr");
        assert_eq!(json[1]["document"]["origin"]["confidence_bps"], 9234);
        assert_eq!(json[1]["document"]["locator"]["owner"]["page"], 4);
        Ok(())
    }

    #[test]
    fn graph_model_preserves_code_graph_v1_source_anchor() -> Result<(), Box<dyn std::error::Error>>
    {
        let document: GraphDocument = serde_json::from_value(json!({
            "nodes": [{
                "id": "sha256:8cfda4",
                "name": "metavoice.rs",
                "kind": "file",
                "language": "rust",
                "source": {
                    "file": "candle-transformers/src/models/metavoice.rs",
                    "startByte": 1842,
                    "endByte": 1976,
                    "startLine": 61,
                    "startColumn": 4,
                    "endLine": 66,
                    "endColumn": 5
                }
            }],
            "links": []
        }))?;
        let communities: Communities = BTreeMap::from([(0, vec!["sha256:8cfda4".to_owned()])]);
        let model = graph_view_model(
            &document,
            &communities,
            "Code Graph v1",
            &HtmlOptions::default(),
            false,
        );

        assert_eq!(
            serde_json::to_value(&model.nodes[0].source)?,
            json!({
                "file": "candle-transformers/src/models/metavoice.rs",
                "startLine": 61,
                "endLine": 66,
                "startByte": 1842,
                "endByte": 1976
            })
        );
        Ok(())
    }

    #[test]
    fn graph_model_preserves_aggregated_edge_confidence() -> Result<(), Box<dyn std::error::Error>>
    {
        let document: GraphDocument = serde_json::from_value(json!({
            "nodes": [
                {"id": "0", "label": "Core"},
                {"id": "1", "label": "Data"}
            ],
            "links": [{
                "source": "0",
                "target": "1",
                "relation": "2 cross-community edges",
                "confidence": "AGGREGATED",
                "weight": 2
            }]
        }))?;
        let communities: Communities =
            BTreeMap::from([(0, vec!["0".into()]), (1, vec!["1".into()])]);
        let model = graph_view_model(
            &document,
            &communities,
            "Aggregate",
            &HtmlOptions::default(),
            true,
        );

        assert_eq!(model.edges[0].relation, "2 cross-community edges");
        assert_eq!(model.edges[0].weight, Some(2));
        assert_eq!(model.edges[0].confidence.as_deref(), Some("aggregated"));
        assert!(model.edges[0].relationship_site.is_none());
        Ok(())
    }

    #[test]
    fn graph_model_preserves_edge_relationship_source_anchor()
    -> Result<(), Box<dyn std::error::Error>> {
        let document: GraphDocument = serde_json::from_value(json!({
            "nodes": [
                {"id": "caller", "label": "caller"},
                {"id": "callee", "label": "callee"}
            ],
            "links": [{
                "id": "caller-callee",
                "source": "caller",
                "target": "callee",
                "kind": "calls",
                "relationshipSite": {
                    "file": "src/main.rs",
                    "startByte": 142,
                    "endByte": 150,
                    "startLine": 7,
                    "startColumn": 12,
                    "endLine": 7,
                    "endColumn": 20
                },
                "evidence": [{
                    "origin": "heuristic",
                    "confidence": "inferred"
                }]
            }]
        }))?;
        let communities: Communities =
            BTreeMap::from([(0, vec!["caller".to_owned(), "callee".to_owned()])]);
        let model = graph_view_model(
            &document,
            &communities,
            "Relationships",
            &HtmlOptions::default(),
            false,
        );

        assert_eq!(model.edges[0].relation, "calls");
        assert_eq!(model.edges[0].confidence.as_deref(), Some("inferred"));
        assert_eq!(
            serde_json::to_value(&model.edges[0])?["relationshipSite"],
            json!({
                "file": "src/main.rs",
                "startLine": 7,
                "endLine": 7,
                "startByte": 142,
                "endByte": 150
            })
        );
        Ok(())
    }

    #[test]
    fn effective_model_recomputes_communities_and_discards_base_labels()
    -> Result<(), Box<dyn std::error::Error>> {
        let effective = serde_json::from_str::<compass_agent_graph::EffectiveGraph>(include_str!(
            "../../../fixtures/contracts/agent-graph/effective-v1.json"
        ))?;
        let stale_labels = BTreeMap::from([(99, "Stale base community".to_owned())]);
        let model = effective_graph_view_model(
            &effective,
            "Effective",
            &HtmlOptions {
                community_labels: Some(&stale_labels),
                ..HtmlOptions::default()
            },
        )?;

        assert!(model.communities.is_empty());
        assert_eq!(
            model
                .effective_graph
                .as_ref()
                .map(|context| &context.effective_identity),
            Some(&effective.effective_identity)
        );
        Ok(())
    }

    #[test]
    fn shared_html_embeds_community_models_as_inert_safe_json()
    -> Result<(), Box<dyn std::error::Error>> {
        let model = GraphViewModel {
            schema: GRAPH_VIEWER_SCHEMA,
            title: "Overview".to_owned(),
            stats: GraphViewStats {
                nodes: 0,
                edges: 0,
                communities: 0,
                aggregated: true,
            },
            nodes: Vec::new(),
            edges: Vec::new(),
            communities: Vec::new(),
            hyperedges: Vec::new(),
            effective_graph: None,
        };
        let mut detail = model.clone();
        detail.title = "</script><script>alert(1)</script>".to_owned();
        detail.stats.aggregated = false;
        let html = shared_viewer_html_with_communities(&model, &BTreeMap::from([(7, detail)]))?;

        assert!(html.contains("data-compass-community=\"7\""));
        assert!(html.contains("\\u003c/script\\u003e"));
        assert!(!html.contains("<script>alert(1)</script>"));
        Ok(())
    }
}
