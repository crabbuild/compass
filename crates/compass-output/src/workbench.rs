use std::collections::BTreeMap;
use std::path::Path;

use compass_analysis::UniversalCallGraphResponse;
use compass_model::query_contract::CodeQueryResponse;
use serde::Serialize;

use crate::{CallflowViewModel, GraphViewModel, OutputError};

pub const WORKBENCH_SCHEMA: &str = "compass.viewer.workbench/1";

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum WorkbenchCoverageStatus {
    Complete,
    Summary,
    Partial,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkbenchCoverage {
    pub status: WorkbenchCoverageStatus,
    pub truncated: bool,
    pub nodes: usize,
    pub edges: usize,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub limitations: Vec<String>,
}

impl WorkbenchCoverage {
    #[must_use]
    pub fn graph(model: &GraphViewModel) -> Self {
        Self {
            status: if model.stats.aggregated {
                WorkbenchCoverageStatus::Summary
            } else {
                WorkbenchCoverageStatus::Complete
            },
            truncated: false,
            nodes: model.stats.nodes,
            edges: model.stats.edges,
            limitations: if model.stats.aggregated {
                vec!["The repository overview is aggregated by community.".to_owned()]
            } else {
                Vec::new()
            },
        }
    }

    #[must_use]
    pub fn bounded(nodes: usize, edges: usize, truncated: bool) -> Self {
        Self {
            status: if truncated {
                WorkbenchCoverageStatus::Partial
            } else {
                WorkbenchCoverageStatus::Complete
            },
            truncated,
            nodes,
            edges,
            limitations: if truncated {
                vec!["The selected view reached its configured node or edge bound.".to_owned()]
            } else {
                Vec::new()
            },
        }
    }
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkbenchModel {
    pub schema: &'static str,
    pub title: String,
    pub graph_identity: String,
    pub default_view: String,
    pub views: Vec<WorkbenchView>,
}

impl WorkbenchModel {
    #[must_use]
    pub fn new(
        title: impl Into<String>,
        graph_identity: impl Into<String>,
        views: Vec<WorkbenchView>,
    ) -> Self {
        let default_view = views
            .first()
            .map_or_else(String::new, |view| view.id.clone());
        Self {
            schema: WORKBENCH_SCHEMA,
            title: title.into(),
            graph_identity: graph_identity.into(),
            default_view,
            views,
        }
    }
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkbenchView {
    pub id: String,
    pub title: String,
    pub description: String,
    pub coverage: WorkbenchCoverage,
    #[serde(flatten)]
    pub content: WorkbenchViewContent,
}

#[derive(Clone, Debug, Serialize)]
#[serde(
    tag = "kind",
    rename_all = "snake_case",
    rename_all_fields = "camelCase"
)]
pub enum WorkbenchViewContent {
    Code {
        model: GraphViewModel,
        community_details: BTreeMap<usize, GraphViewModel>,
    },
    Call {
        root: String,
        graph: UniversalCallGraphResponse,
    },
    Impact {
        root: String,
        result: CodeQueryResponse,
    },
    Architecture {
        model: CallflowViewModel,
    },
    History {
        base_revision: String,
        target_revision: String,
        before: GraphViewModel,
        after: GraphViewModel,
    },
    Affected {
        root: String,
        relations: Vec<String>,
        depth: usize,
        model: GraphViewModel,
    },
    Artifact {
        lens: crate::ArtifactLens,
        relations: Vec<String>,
        model: GraphViewModel,
    },
}

pub fn workbench_html_document(model: &WorkbenchModel) -> Result<String, serde_json::Error> {
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
<meta http-equiv="Content-Security-Policy" content="default-src 'none'; img-src data:; style-src 'unsafe-inline'; script-src 'unsafe-inline';">
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

pub fn write_workbench_html(
    model: &WorkbenchModel,
    output_path: impl AsRef<Path>,
) -> Result<(), OutputError> {
    let html = workbench_html_document(model)?;
    compass_files::write_text_atomic(output_path, &html)?;
    Ok(())
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
    use super::*;

    #[test]
    fn workbench_document_escapes_embedded_json_and_title() -> Result<(), Box<dyn std::error::Error>>
    {
        let model = WorkbenchModel::new(
            "A < B",
            "sha256:test",
            vec![WorkbenchView {
                id: "code".to_owned(),
                title: "Code".to_owned(),
                description: "Repository structure".to_owned(),
                coverage: WorkbenchCoverage::bounded(0, 0, false),
                content: WorkbenchViewContent::Code {
                    model: GraphViewModel {
                        schema: crate::GRAPH_VIEWER_SCHEMA,
                        title: "</script>".to_owned(),
                        stats: crate::GraphViewStats {
                            nodes: 0,
                            edges: 0,
                            communities: 0,
                            aggregated: false,
                        },
                        nodes: Vec::new(),
                        edges: Vec::new(),
                        communities: Vec::new(),
                        hyperedges: Vec::new(),
                    },
                    community_details: BTreeMap::new(),
                },
            }],
        );
        let html = workbench_html_document(&model)?;
        assert!(html.contains("Compass — A &lt; B"));
        assert!(!html.contains("</script>\""));
        assert!(html.contains("compass.viewer.workbench/1"));
        assert!(html.contains("\"communityDetails\""));
        assert!(!html.contains("\"community_details\""));
        Ok(())
    }
}
