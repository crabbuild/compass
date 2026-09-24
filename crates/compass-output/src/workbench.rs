use std::collections::BTreeMap;
use std::path::Path;

use compass_analysis::UniversalCallGraphResponse;
use compass_model::query_contract::CodeQueryResponse;
use serde::Serialize;
use url::Url;

use crate::{ArchitectureViewModel, GraphViewModel, OutputError};

pub const WORKBENCH_SCHEMA: &str = "compass.viewer.workbench/1";

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum SourceProvider {
    Github,
    Gitlab,
    Bitbucket,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SourceNavigation {
    pub provider: SourceProvider,
    pub repository_url: String,
    pub revision: String,
}

impl SourceNavigation {
    /// Build immutable browser navigation metadata from a recognized Git forge remote.
    #[must_use]
    pub fn from_git_remote(remote: &str, revision: &str) -> Option<Self> {
        if !is_full_commit(revision) {
            return None;
        }
        let (host, path, web_port) = parse_remote(remote)?;
        let provider = source_provider(&host)?;
        let port = web_port.map_or_else(String::new, |port| format!(":{port}"));
        Some(Self {
            provider,
            repository_url: format!("https://{host}{port}/{path}"),
            revision: revision.to_ascii_lowercase(),
        })
    }
}

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
    /// Published hierarchy levels the view carries, when a build published one.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub hierarchy_levels: Option<usize>,
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
            hierarchy_levels: None,
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
            hierarchy_levels: None,
            limitations: if truncated {
                vec!["The selected view reached its configured node or edge bound.".to_owned()]
            } else {
                Vec::new()
            },
        }
    }

    /// Record how many published levels this view carries.
    #[must_use]
    pub fn with_hierarchy_levels(mut self, levels: usize) -> Self {
        self.hierarchy_levels = Some(levels);
        self
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
// Keep the established public model types and serialized contract intact. The
// effective-graph context can make history the largest variant, but boxing it
// here would be a source-breaking public API change.
#[allow(clippy::large_enum_variant)]
pub enum WorkbenchViewContent {
    Code {
        model: GraphViewModel,
        community_details: BTreeMap<usize, GraphViewModel>,
        /// Published level navigation, when the build published a hierarchy.
        #[serde(skip_serializing_if = "Option::is_none")]
        hierarchy: Option<crate::hierarchy_view::CommunityHierarchyView>,
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
        model: ArchitectureViewModel,
    },
    History {
        base_revision: String,
        target_revision: String,
        before: Box<GraphViewModel>,
        after: Box<GraphViewModel>,
        /// Bounded comparison of the two generations' community hierarchies,
        /// absent when either side published none.
        #[serde(skip_serializing_if = "Option::is_none")]
        hierarchy_diff: Option<Box<compass_semantic_diff::HierarchyDiff>>,
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
    workbench_html_document_with_source_navigation(model, None)
}

pub fn workbench_html_document_with_source_navigation(
    model: &WorkbenchModel,
    source_navigation: Option<&SourceNavigation>,
) -> Result<String, serde_json::Error> {
    let model_json = serde_json::to_string(model)?
        .replace('<', "\\u003c")
        .replace('>', "\\u003e")
        .replace('&', "\\u0026");
    let source_navigation = source_navigation
        .map(serde_json::to_string)
        .transpose()?
        .map(|value| {
            format!(
                r#"<script id="compass-source-navigation" type="application/json">{}</script>"#,
                escape_embedded_json(&value)
            )
        })
        .unwrap_or_default();
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
{source_navigation}
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

pub fn write_workbench_html_with_source_navigation(
    model: &WorkbenchModel,
    source_navigation: Option<&SourceNavigation>,
    output_path: impl AsRef<Path>,
) -> Result<(), OutputError> {
    let html = workbench_html_document_with_source_navigation(model, source_navigation)?;
    compass_files::write_text_atomic(output_path, &html)?;
    Ok(())
}

fn escape_embedded_json(value: &str) -> String {
    value
        .replace('<', "\\u003c")
        .replace('>', "\\u003e")
        .replace('&', "\\u0026")
}

fn is_full_commit(revision: &str) -> bool {
    matches!(revision.len(), 40 | 64) && revision.bytes().all(|byte| byte.is_ascii_hexdigit())
}

fn source_provider(host: &str) -> Option<SourceProvider> {
    if host == "github.com" {
        Some(SourceProvider::Github)
    } else if host == "gitlab.com" {
        Some(SourceProvider::Gitlab)
    } else if host == "bitbucket.org" {
        Some(SourceProvider::Bitbucket)
    } else {
        None
    }
}

fn parse_remote(remote: &str) -> Option<(String, String, Option<u16>)> {
    let remote = remote.trim();
    if remote.is_empty()
        || remote
            .chars()
            .any(|character| matches!(character, '\0' | '\n' | '\r'))
    {
        return None;
    }
    let (host, path, port) = if remote.contains("://") {
        let parsed = Url::parse(remote).ok()?;
        if !matches!(parsed.scheme(), "http" | "https" | "ssh" | "git") {
            return None;
        }
        let host = parsed.host_str()?.to_ascii_lowercase();
        let port = matches!(parsed.scheme(), "http" | "https")
            .then(|| parsed.port())
            .flatten();
        (host, parsed.path().to_owned(), port)
    } else {
        let without_user = remote.rsplit_once('@').map_or(remote, |(_, value)| value);
        let (host, path) = without_user.split_once(':')?;
        (host.to_ascii_lowercase(), path.to_owned(), None)
    };
    let path = path
        .trim_matches('/')
        .strip_suffix(".git")
        .unwrap_or(path.trim_matches('/'));
    if host.is_empty()
        || path.is_empty()
        || path
            .split('/')
            .any(|segment| segment.is_empty() || matches!(segment, "." | ".."))
    {
        return None;
    }
    Some((host, path.to_owned(), port))
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

    fn empty_model(title: &str) -> GraphViewModel {
        GraphViewModel {
            schema: crate::GRAPH_VIEWER_SCHEMA,
            title: title.to_owned(),
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
            effective_graph: None,
        }
    }

    #[test]
    fn history_view_publishes_a_hierarchy_diff_only_when_one_exists() {
        let level_signature = {
            use sha2::{Digest, Sha256};
            let digest = format!("{:x}", Sha256::digest(b"0123456789abcdef\0"));
            digest.chars().take(16).collect::<String>()
        };
        let model = compass_graph::CommunityHierarchy::new(
            "generation".to_owned(),
            format!("sha256:{}", "0".repeat(64)),
            compass_graph::CommunityHierarchyDraft {
                identity: compass_graph::CommunityIdentity {
                    algorithm: compass_graph::QUALITY_CLUSTER_ALGORITHM.to_owned(),
                    topology: compass_graph::QUALITY_CLUSTER_TOPOLOGY.to_owned(),
                    quality: compass_graph::QUALITY_CLUSTER_QUALITY.to_owned(),
                    selector: compass_graph::QUALITY_CLUSTER_SELECTOR.to_owned(),
                    seed: compass_graph::COMPATIBILITY_CLUSTER_SEED,
                    limits: compass_graph::QUALITY_CLUSTER_LIMITS.to_owned(),
                },
                limits: compass_graph::CommunityLimits::default(),
                budget_identity: compass_graph::COMMUNITY_HIERARCHY_BUDGET.to_owned(),
                merge_policy: compass_graph::COMMUNITY_HIERARCHY_MERGE_POLICY.to_owned(),
                signature_algorithm: compass_graph::COMMUNITY_HIERARCHY_SIGNATURE_ALGORITHM
                    .to_owned(),
                budget: compass_graph::HierarchyBudget::default(),
                boundary_kinds: vec!["route".to_owned()],
                finest_community_count: 1,
                finest_signature: format!("sha256:{}", "1".repeat(64)),
                budget_satisfied: true,
                levels: vec![compass_graph::HierarchyLevel {
                    level: 0,
                    signature: level_signature.clone(),
                    merge: compass_graph::LevelMerge::Relationship,
                    resolution: Some(1.0),
                    merge_evidence: BTreeMap::new(),
                    group_count: 1,
                    groups: vec![compass_graph::HierarchyGroup {
                        index: 0,
                        id: "h0-0123456789abcdef".to_owned(),
                        signature: "0123456789abcdef".to_owned(),
                        community: Some(0),
                        label: compass_graph::HierarchyLabel {
                            text: "src".to_owned(),
                            rule: compass_graph::HierarchyLabelRule::DominantDirectory,
                            generic: false,
                            evidence: BTreeMap::from([(
                                "value".to_owned(),
                                serde_json::json!("src"),
                            )]),
                        },
                        member_count: 1,
                        child_indices: Vec::new(),
                        quality: compass_graph::GroupQuality {
                            cohesion: 1.0,
                            conductance: 0.0,
                            boundary_kinds: BTreeMap::new(),
                        },
                    }],
                }],
            },
        )
        .ok()
        .expect("fixture hierarchy builds");
        let communities = compass_graph::Communities::from([(0usize, vec!["a".to_owned()])]);
        let diff = compass_semantic_diff::compare_hierarchies(
            &model,
            &communities,
            &model,
            &communities,
            &compass_graph::ReconcilePolicy::default(),
        )
        .ok()
        .expect("comparing a hierarchy with itself succeeds");

        let view = WorkbenchViewContent::History {
            base_revision: "aaa".to_owned(),
            target_revision: "bbb".to_owned(),
            before: Box::new(empty_model("before")),
            after: Box::new(empty_model("after")),
            hierarchy_diff: Some(Box::new(diff)),
        };
        let value = serde_json::to_value(&view).unwrap_or_default();
        assert_eq!(value["kind"], "history");
        assert_eq!(
            value["hierarchyDiff"]["schema"],
            "compass.community-hierarchy-diff/1"
        );

        let without = WorkbenchViewContent::History {
            base_revision: "aaa".to_owned(),
            target_revision: "bbb".to_owned(),
            before: Box::new(empty_model("before")),
            after: Box::new(empty_model("after")),
            hierarchy_diff: None,
        };
        let value = serde_json::to_value(&without).unwrap_or_default();
        assert!(
            value.get("hierarchyDiff").is_none(),
            "an unavailable comparison is absent, not empty"
        );
    }

    #[test]
    fn source_navigation_normalizes_recognized_forge_remotes() {
        let revision = "0123456789abcdef0123456789abcdef01234567";
        assert_eq!(
            SourceNavigation::from_git_remote("git@github.com:crabbuild/compass.git", revision,),
            Some(SourceNavigation {
                provider: SourceProvider::Github,
                repository_url: "https://github.com/crabbuild/compass".to_owned(),
                revision: revision.to_owned(),
            })
        );
        assert_eq!(
            SourceNavigation::from_git_remote(
                "ssh://git@bitbucket.org/platform/tools-compass.git",
                revision,
            )
            .map(|navigation| navigation.repository_url),
            Some("https://bitbucket.org/platform/tools-compass".to_owned())
        );
        assert!(
            SourceNavigation::from_git_remote("https://example.com/acme/compass.git", revision,)
                .is_none()
        );
        assert!(
            SourceNavigation::from_git_remote("https://bitbucket.org/acme/compass.git", "main",)
                .is_none()
        );
    }

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
                        effective_graph: None,
                    },
                    community_details: BTreeMap::new(),
                    hierarchy: None,
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
