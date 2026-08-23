//! Compatibility entry points for the architecture export.
//!
//! The command names retain `callflow` for CLI compatibility, but every export
//! is backed by the typed architecture projection. This module deliberately
//! contains no independent grouping, naming, or relationship semantics.

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use compass_graph::Communities;
use compass_model::GraphDocument;
use serde::{Deserialize, Serialize};

use crate::{
    ArchitectureOverlay, ArchitectureOverlayGroup, ArchitectureProjectionInput,
    ArchitectureProjectionOptions, ArchitectureScope, ArchitectureViewModel, OutputError,
    WorkbenchCoverage, WorkbenchModel, WorkbenchView, WorkbenchViewContent, project_architecture,
    workbench_html_document,
};

/// Legacy section input accepted as an adapter to the versioned overlay.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CallflowSection {
    pub id: String,
    pub name: String,
    pub communities: Vec<String>,
}

#[derive(Clone, Debug)]
pub struct CallflowOptions<'a> {
    pub community_labels: Option<&'a BTreeMap<usize, String>>,
    pub sections: Option<&'a [CallflowSection]>,
    pub overlay: Option<&'a ArchitectureOverlay>,
    pub report: &'a str,
    pub project_name: &'a str,
    pub built_at_commit: Option<&'a str>,
    pub language: &'a str,
    pub max_sections: usize,
    pub diagram_scale: f64,
    pub max_diagram_nodes: usize,
    pub max_diagram_edges: usize,
    pub generated_at: Option<&'a str>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CallflowExport {
    pub groups: usize,
    pub overview_groups: usize,
    pub relationships: usize,
    pub overview_routes: usize,
}

impl Default for CallflowOptions<'_> {
    fn default() -> Self {
        Self {
            community_labels: None,
            sections: None,
            overlay: None,
            report: "",
            project_name: "Project",
            built_at_commit: None,
            language: "auto",
            max_sections: 15,
            diagram_scale: 1.0,
            max_diagram_nodes: 18,
            max_diagram_edges: 24,
            generated_at: None,
        }
    }
}

/// Returns the bounded overview as legacy section records without inventing an
/// overflow group. New consumers should use [`project_architecture`] directly.
#[must_use]
pub fn derive_callflow_sections(
    document: &GraphDocument,
    communities: &Communities,
    labels: Option<&BTreeMap<usize, String>>,
    _language: &str,
    max_sections: usize,
) -> Vec<CallflowSection> {
    let default_options = ArchitectureProjectionOptions::default();
    let options = ArchitectureProjectionOptions {
        scopes: BTreeSet::from([ArchitectureScope::Production]),
        limits: crate::ArchitectureProjectionLimits {
            max_overview_groups: max_sections.max(2),
            ..default_options.limits
        },
        ..default_options
    };
    let Ok(model) = project_architecture(
        ArchitectureProjectionInput {
            document,
            communities,
            community_labels: labels,
            overlay: None,
            project_name: "Project",
            built_at_commit: None,
            generated_at: None,
        },
        &options,
    ) else {
        return Vec::new();
    };
    let Some(projection) = model.projections.first() else {
        return Vec::new();
    };
    let shown = projection
        .overview_group_ids
        .iter()
        .collect::<BTreeSet<_>>();
    projection
        .groups
        .iter()
        .filter(|group| shown.contains(&group.id))
        .map(|group| CallflowSection {
            id: group.id.clone(),
            name: group.name.value.clone(),
            communities: group.community_ids.iter().map(usize::to_string).collect(),
        })
        .collect()
}

pub fn callflow_html_document(
    document: &GraphDocument,
    communities: &Communities,
    options: &CallflowOptions<'_>,
) -> Result<String, OutputError> {
    let model = architecture_model(document, communities, options)?;
    architecture_html_for_model(model, options)
}

fn architecture_html_for_model(
    model: ArchitectureViewModel,
    options: &CallflowOptions<'_>,
) -> Result<String, OutputError> {
    if model.projections.is_empty() {
        return Err(OutputError::NoCallflowSections);
    }
    let graph_identity = options.built_at_commit.map_or_else(
        || "architecture:unpublished".to_owned(),
        |value| format!("git:{value}"),
    );
    let workbench = WorkbenchModel::new(
        model.title.clone(),
        graph_identity,
        vec![WorkbenchView {
            id: "architecture".to_owned(),
            title: "Architecture".to_owned(),
            description: "Source-scoped, typed subsystem relationships".to_owned(),
            // The full typed inventory is embedded. Overview omissions are a
            // presentation budget and are disclosed by the architecture model;
            // they do not make extraction partial.
            coverage: WorkbenchCoverage::bounded(
                model.statistics.nodes,
                model.statistics.relationships,
                false,
            ),
            content: WorkbenchViewContent::Architecture { model },
        }],
    );
    workbench_html_document(&workbench).map_err(OutputError::from)
}

pub fn write_callflow_html(
    document: &GraphDocument,
    communities: &Communities,
    output_path: impl AsRef<Path>,
    options: &CallflowOptions<'_>,
) -> Result<CallflowExport, OutputError> {
    let model = architecture_model(document, communities, options)?;
    let projection = model
        .projections
        .first()
        .ok_or(OutputError::NoCallflowSections)?;
    let result = CallflowExport {
        groups: projection.groups.len(),
        overview_groups: projection.overview_group_ids.len(),
        relationships: model.statistics.relationships,
        overview_routes: projection.overview_route_ids.len(),
    };
    let html = architecture_html_for_model(model, options)?;
    compass_files::write_text_atomic(output_path, &html)?;
    Ok(result)
}

fn architecture_model(
    document: &GraphDocument,
    communities: &Communities,
    options: &CallflowOptions<'_>,
) -> Result<ArchitectureViewModel, OutputError> {
    if document.nodes.is_empty() {
        return Err(OutputError::EmptyCallflowGraph);
    }
    let adapted_overlay = options
        .sections
        .filter(|_| options.overlay.is_none())
        .map(legacy_overlay);
    let defaults = ArchitectureProjectionOptions::default();
    let projection_options = ArchitectureProjectionOptions {
        limits: crate::ArchitectureProjectionLimits {
            max_overview_groups: options.max_sections.max(2),
            ..defaults.limits
        },
        ..defaults
    };
    project_architecture(
        ArchitectureProjectionInput {
            document,
            communities,
            community_labels: options.community_labels,
            overlay: options.overlay.or(adapted_overlay.as_ref()),
            project_name: options.project_name,
            built_at_commit: options.built_at_commit,
            generated_at: options.generated_at,
        },
        &projection_options,
    )
    .map_err(|error| OutputError::InvalidArchitectureProjection(error.to_string()))
}

fn legacy_overlay(sections: &[CallflowSection]) -> ArchitectureOverlay {
    ArchitectureOverlay {
        schema: crate::ARCHITECTURE_OVERLAY_SCHEMA.to_owned(),
        source_rules: Vec::new(),
        groups: sections
            .iter()
            .filter(|section| section.id != "overview")
            .map(|section| ArchitectureOverlayGroup {
                id: section.id.clone(),
                name: section.name.clone(),
                communities: section
                    .communities
                    .iter()
                    .filter_map(|value| value.parse::<usize>().ok())
                    .collect(),
                path_prefixes: Vec::new(),
                pin: false,
            })
            .collect(),
    }
}

#[cfg(test)]
mod tests {
    use std::error::Error;

    use serde_json::json;

    use super::*;

    #[test]
    fn html_embeds_only_the_architecture_contract_and_escapes_script_terminators()
    -> Result<(), Box<dyn Error>> {
        let document: GraphDocument = serde_json::from_value(json!({
            "graph": {},
            "nodes": [{
                "id":"node",
                "label":"</script><script>alert(1)</script>",
                "source_file":"src/lib.rs"
            }],
            "links": []
        }))?;
        let communities = BTreeMap::from([(0, vec!["node".to_owned()])]);
        let html = callflow_html_document(
            &document,
            &communities,
            &CallflowOptions {
                project_name: "Hostile </script>",
                ..CallflowOptions::default()
            },
        )?;
        assert!(html.contains("compass.viewer.architecture/1"));
        assert!(!html.contains("compass.viewer.callflow/1"));
        assert!(!html.contains("</script><script>alert(1)</script>"));
        assert!(html.contains(r#""coverage":{"status":"complete","truncated":false"#));
        Ok(())
    }
}
