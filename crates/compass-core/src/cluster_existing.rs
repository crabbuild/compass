use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use compass_files::{BuildGuard, write_atomic_with_digest, write_json_atomic, write_text_atomic};
use compass_graph::{
    ClusterOptions, Communities, CommunityHierarchy, CommunityLimits, CommunityProfile,
    CommunityQualityArtifact, CommunityRequest, GodNode, HierarchyBudget, HierarchyRequest,
    ResolutionPolicy, blind_spot_report, build_communities, build_community_hierarchy, cluster,
    community_member_signatures, god_nodes, label_communities_by_hub,
    remap_communities_to_previous, score_communities, suggest_questions, surprising_connections,
    write_canonical_graph_json,
};
use compass_model::GraphDocument;
use compass_model::GraphError;
use compass_model::code_graph::{CommunityMetadata, GraphDocument as V1GraphDocument};
use compass_output::{
    DetectionSummary, FreshnessBasis, FreshnessStatus, HtmlOptions, JsonExportOptions,
    OrientationHealth, ReportOptions, TokenCost, agent_orientation_with_blind_spots,
    backup_if_protected_to, graph_artifact_identity, render_agent_report_markdown,
    render_orientation_json, write_html, write_json,
};
use serde_json::{Value, json};

use crate::pipeline::{git_commit, remove_if_exists, write_graph_overview_artifact};
use crate::{CoreError, load_learning_for_report};

#[derive(Clone, Debug)]
pub struct ClusterExistingOptions {
    pub graph_path: PathBuf,
    pub output_dir: PathBuf,
    pub root: PathBuf,
    pub no_viz: bool,
    pub no_label: bool,
    pub resolution: f64,
    pub resolution_explicit: bool,
    pub exclude_hubs: Option<f64>,
    pub min_community_size: usize,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ClusterExistingResult {
    pub nodes: usize,
    pub edges: usize,
    pub communities: usize,
    pub labels_reused: usize,
    pub html_written: bool,
    pub load_warning: Option<String>,
    pub backup_message: Option<String>,
    pub backup_warning: Option<String>,
    pub timings: ClusterExistingTimings,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ClusterExistingTimings {
    pub load: Duration,
    pub cluster: Duration,
    pub analyze: Duration,
    pub label: Duration,
    pub report: Duration,
    pub export: Duration,
    pub total: Duration,
}

pub struct ClusterLabelContext<'a> {
    pub document: &'a GraphDocument,
    pub communities: &'a Communities,
    pub hub_labels: &'a BTreeMap<usize, String>,
    pub saved_labels: &'a BTreeMap<usize, String>,
    pub saved_signatures: &'a BTreeMap<usize, String>,
    pub signatures: &'a BTreeMap<usize, String>,
    pub gods: &'a [GodNode],
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ClusterLabelSelection {
    pub labels: BTreeMap<usize, String>,
    pub labels_reused: usize,
    pub token_cost: TokenCost,
}

pub fn cluster_existing_graph(
    options: &ClusterExistingOptions,
) -> Result<ClusterExistingResult, CoreError> {
    cluster_existing_graph_with_labeler(options, |context| {
        let mut labels_reused = 0;
        let labels = context
            .communities
            .keys()
            .map(|community| {
                if options.no_label {
                    return (*community, format!("Community {community}"));
                }
                let reusable =
                    context.saved_signatures.get(community) == context.signatures.get(community);
                let label = if reusable {
                    context.saved_labels.get(community).cloned().inspect(|_| {
                        labels_reused += 1;
                    })
                } else {
                    None
                }
                .unwrap_or_else(|| context.hub_labels[community].clone());
                (*community, label)
            })
            .collect();
        ClusterLabelSelection {
            labels,
            labels_reused,
            token_cost: TokenCost::default(),
        }
    })
}

pub fn cluster_existing_graph_with_labeler<F>(
    options: &ClusterExistingOptions,
    labeler: F,
) -> Result<ClusterExistingResult, CoreError>
where
    F: FnOnce(&ClusterLabelContext<'_>) -> ClusterLabelSelection,
{
    let total_started = Instant::now();
    let load_started = Instant::now();
    let load_warning = None;
    let (mut typed_document, document) =
        match V1GraphDocument::load_for_recluster_with_artifact_digest(&options.graph_path) {
            Ok((typed, _artifact_digest)) => {
                let legacy = typed.to_legacy_document()?;
                (Some(typed), legacy)
            }
            Err(GraphError::UnsupportedGraphSchema { found: None }) => (
                None,
                GraphDocument::load_for_recluster(&options.graph_path)?,
            ),
            Err(error) => return Err(error.into()),
        };
    let mut clustering_document = Some(document);
    {
        let document = clustering_document
            .as_mut()
            .ok_or_else(|| CoreError::InvalidBuildState("graph document missing".to_owned()))?;
        normalize_recluster_document(document);
        if document.nodes.is_empty() {
            return Err(CoreError::EmptyGraph);
        }
    };
    let document = clustering_document
        .as_ref()
        .ok_or_else(|| CoreError::InvalidBuildState("graph document missing".to_owned()))?;
    let load_elapsed = load_started.elapsed();
    let previous = document
        .nodes
        .iter()
        .filter_map(|node| {
            let community = node
                .attributes
                .get("community")?
                .as_u64()
                .and_then(|value| usize::try_from(value).ok())?;
            Some((node.id.clone(), community))
        })
        .collect::<HashMap<_, _>>();
    let cluster_started = Instant::now();
    let community_limits = CommunityLimits::default();
    let (communities, quality_evidence) = if let Some(typed) = typed_document.as_ref() {
        let changed_sources = BTreeSet::new();
        let result = build_communities(
            typed,
            &CommunityRequest {
                profile: CommunityProfile::QualityV1,
                resolution: ResolutionPolicy::Fixed(options.resolution),
                exclude_hubs_percentile: options.exclude_hubs,
                previous: (!previous.is_empty()).then_some(&previous),
                incremental: false,
                changed_sources: &changed_sources,
                limits: community_limits,
            },
        )?;
        // The hierarchy is derived from the published partition and the same
        // typed topology projection, so it is built here rather than re-read
        // from the artifacts it describes.
        let hierarchy = build_community_hierarchy(
            typed,
            &result.communities,
            &HierarchyRequest {
                identity: &result.identity,
                limits: &community_limits,
                resolution: result.quality.resolution,
                budget: HierarchyBudget::default(),
            },
        )?;
        (
            result.communities,
            Some((
                typed.graph.build.generation_id.clone(),
                result.identity,
                result.quality,
                hierarchy,
            )),
        )
    } else {
        let fresh = cluster(
            document,
            ClusterOptions {
                resolution: options.resolution,
                exclude_hubs_percentile: options.exclude_hubs,
            },
        );
        let communities = if previous.is_empty() {
            fresh
        } else {
            remap_communities_to_previous(&fresh, &previous)
        };
        (communities, None)
    };
    let cluster_elapsed = cluster_started.elapsed();
    let analyze_started = Instant::now();
    let hub_labels = label_communities_by_hub(document, &communities);
    let signatures = community_member_signatures(&communities);
    let saved_labels = load_usize_string_map(&options.output_dir.join("labels.json"));
    let saved_signatures = load_usize_string_map(&options.output_dir.join("labels.json.sig"));
    let cluster_gods = god_nodes(document, 10);
    let analyze_elapsed = analyze_started.elapsed();
    let label_started = Instant::now();
    let selection = labeler(&ClusterLabelContext {
        document,
        communities: &communities,
        hub_labels: &hub_labels,
        saved_labels: &saved_labels,
        saved_signatures: &saved_signatures,
        signatures: &signatures,
        gods: &cluster_gods,
    });
    let label_elapsed = label_started.elapsed();
    let labels = selection.labels;
    let report_started = Instant::now();
    if let Some(typed) = &mut typed_document {
        let node_communities = communities
            .iter()
            .flat_map(|(community, members)| {
                members
                    .iter()
                    .map(move |member| (member.as_str(), *community))
            })
            .collect::<HashMap<_, _>>();
        for node in &mut typed.nodes {
            let Some(&community_index) = node_communities.get(node.id.as_str()) else {
                continue;
            };
            node.community = Some(CommunityMetadata {
                id: u64::try_from(community_index).map_err(|_| {
                    CoreError::InvalidBuildState("community ID exceeds u64".to_owned())
                })?,
                label: labels.get(&community_index).cloned(),
                score: None,
                color: None,
            });
        }
    }
    if typed_document.is_some() {
        clustering_document = None;
    }
    let exact_typed_projection = typed_document
        .as_ref()
        .map(V1GraphDocument::to_legacy_document)
        .transpose()?;
    let published_document = exact_typed_projection
        .as_ref()
        .or(clustering_document.as_ref())
        .ok_or_else(|| CoreError::InvalidBuildState("graph document missing".to_owned()))?;
    let cohesion = score_communities(published_document, &communities);
    let gods = god_nodes(published_document, 10);
    let surprises = surprising_connections(published_document, &communities, 5);
    let questions = suggest_questions(published_document, &communities, &labels, 10);
    let blind_spots = blind_spot_report(published_document, &communities, &labels);
    let commit_root = std::env::current_dir().unwrap_or_else(|_| options.root.clone());
    let commit = git_commit(&commit_root);
    let report_root = options.root.to_string_lossy();
    let report_commit = match &typed_document {
        Some(typed) => typed.graph.build.source_commit.clone(),
        None => commit.clone(),
    };
    let report_options = cluster_only_report_options(
        &report_root,
        options.min_community_size,
        report_commit.as_deref(),
    );
    let learning = load_learning_for_report(&options.output_dir.join("graph.json"));
    let mut orientation = agent_orientation_with_blind_spots(
        published_document,
        &communities,
        &cohesion,
        &labels,
        &gods,
        &surprises,
        &DetectionSummary {
            warning: Some("cluster-only mode — file stats not available".to_owned()),
            ..DetectionSummary::default()
        },
        selection.token_cost,
        Some(&questions),
        Some(&blind_spots),
        learning.as_ref(),
        &report_options,
    );
    let export_started = Instant::now();
    let output_container = BuildGuard::output_container_for_artifact(&options.graph_path);
    let backup = backup_if_protected_to(&options.output_dir, &output_container);
    let guard = BuildGuard::begin_excluding(&output_container, &[])?;
    let staging = guard.staging_directory();
    write_json_atomic(
        staging.join("analysis.json"),
        &json!({
            "communities": communities.iter().map(|(key, value)| (key.to_string(), value)).collect::<BTreeMap<_, _>>(),
            "cohesion": cohesion.iter().map(|(key, value)| (key.to_string(), value)).collect::<BTreeMap<_, _>>(),
            "gods": gods,
            "surprises": surprises,
            "questions": questions,
            "blindSpots": blind_spots,
        }),
        true,
    )?;
    let publishes_quality = quality_evidence.is_some();
    let graph_path = staging.join("graph.json");
    let graph_identity = if let Some(typed) = typed_document {
        let receipt = write_atomic_with_digest(&graph_path, |writer| {
            write_canonical_graph_json(&typed, writer).map_err(|source| {
                compass_files::FileError::Io {
                    path: graph_path.clone(),
                    source,
                }
            })
        })?;
        format!("sha256:{}", receipt.sha256)
    } else {
        write_json(
            published_document,
            &communities,
            &graph_path,
            &JsonExportOptions {
                force: false,
                built_at_commit: commit.as_deref(),
                community_labels: Some(&labels),
            },
        )?;
        graph_artifact_identity(&graph_path)?
    };
    if let Some((generation, identity, quality, hierarchy)) = quality_evidence {
        let artifact = CommunityQualityArtifact::new(
            generation.clone(),
            graph_identity.clone(),
            identity,
            community_limits,
            quality,
        )?;
        write_json_atomic(staging.join("community-quality.json"), &artifact, true)?;
        let hierarchy = CommunityHierarchy::new(generation, graph_identity.clone(), hierarchy)?;
        write_json_atomic(staging.join("community-hierarchy.json"), &hierarchy, true)?;
    }
    orientation.evidence_status.artifact_set_identity = Some(graph_identity);
    let report = render_agent_report_markdown(&orientation, report_options.obsidian)?;
    let orientation_json = render_orientation_json(&orientation)?;
    write_text_atomic(staging.join("GRAPH_REPORT.md"), &report)?;
    write_text_atomic(
        staging.join("orientation.json"),
        &format!("{orientation_json}\n"),
    )?;
    let report_elapsed = report_started.elapsed();
    write_python_string_map(staging.join("labels.json"), &labels)?;
    write_python_string_map(staging.join("labels.json.sig"), &signatures)?;
    write_graph_overview_artifact(published_document, &communities, &labels, staging)?;
    let html_path = staging.join("graph.html");
    let html_written = if options.no_viz {
        remove_if_exists(&html_path)?;
        false
    } else {
        let rendered = write_html(
            published_document,
            &communities,
            &html_path,
            &HtmlOptions {
                community_labels: Some(&labels),
                node_limit: Some(5_000),
                ..HtmlOptions::default()
            },
        )?;
        if rendered.is_none() {
            remove_if_exists(&html_path)?;
        }
        rendered.is_some()
    };
    let mut artifacts = vec![
        "graph.json",
        "GRAPH_REPORT.md",
        "orientation.json",
        "analysis.json",
        "labels.json",
        "labels.json.sig",
        "graph-overview.json",
    ];
    if html_written {
        artifacts.push("graph.html");
    }
    if publishes_quality {
        artifacts.push("community-quality.json");
        artifacts.push("community-hierarchy.json");
    }
    guard.commit_with_artifacts(&artifacts)?;
    BuildGuard::publish_root_artifacts(
        &output_container,
        &[
            "GRAPH_REPORT.md",
            "orientation.json",
            "analysis.json",
            "labels.json",
            "labels.json.sig",
            "graph-overview.json",
            "graph.html",
            "graph.json",
            "community-quality.json",
            "community-hierarchy.json",
        ],
        true,
    )?;
    let export_elapsed = export_started.elapsed();
    Ok(ClusterExistingResult {
        nodes: published_document.nodes.len(),
        edges: published_document.links.len(),
        communities: communities.len(),
        labels_reused: selection.labels_reused,
        html_written,
        load_warning,
        backup_message: backup.message,
        backup_warning: backup.warning,
        timings: ClusterExistingTimings {
            load: load_elapsed,
            cluster: cluster_elapsed,
            analyze: analyze_elapsed,
            label: label_elapsed,
            report: report_elapsed,
            export: export_elapsed,
            total: total_started.elapsed(),
        },
    })
}

fn cluster_only_orientation_health() -> OrientationHealth {
    OrientationHealth {
        freshness: FreshnessStatus::Unknown,
        freshness_basis: FreshnessBasis::Unavailable,
        publication: None,
        build_profile: Some("cluster-only".to_owned()),
        corpus_measurements_available: false,
        ..OrientationHealth::default()
    }
}

fn cluster_only_report_options<'a>(
    root: &'a str,
    min_community_size: usize,
    commit: Option<&'a str>,
) -> ReportOptions<'a> {
    let mut options = ReportOptions::new(root);
    options.min_community_size = min_community_size;
    options.built_at_commit = commit;
    options.health = cluster_only_orientation_health();
    options
}

/// Python's cluster-only path deliberately rebuilds extraction JSON through
/// `build_from_json`, which always creates a simple Graph/DiGraph regardless
/// of node-link metadata. Preserve that command-specific contract without
/// weakening the NetworkX multigraph defaults used by query commands.
fn normalize_recluster_document(document: &mut GraphDocument) {
    let mut positions = HashMap::<(String, String), usize>::new();
    let mut links: Vec<compass_model::EdgeRecord> = Vec::new();
    for edge in document.links.drain(..) {
        let key = if document.directed || edge.source <= edge.target {
            (edge.source.clone(), edge.target.clone())
        } else {
            (edge.target.clone(), edge.source.clone())
        };
        if let Some(&position) = positions.get(&key) {
            links[position].attributes.extend(edge.attributes);
        } else {
            positions.insert(key, links.len());
            links.push(edge);
        }
    }
    document.multigraph = false;
    document.links = links;
}

fn load_usize_string_map(path: &Path) -> BTreeMap<usize, String> {
    fs::read(path)
        .ok()
        .and_then(|bytes| serde_json::from_slice::<Value>(&bytes).ok())
        .and_then(|value| {
            value.as_object().map(|object| {
                object
                    .iter()
                    .filter_map(|(key, value)| {
                        Some((key.parse().ok()?, value.as_str()?.to_owned()))
                    })
                    .collect()
            })
        })
        .unwrap_or_default()
}

fn write_python_string_map(
    path: PathBuf,
    values: &BTreeMap<usize, String>,
) -> Result<(), CoreError> {
    let mut fields = Vec::with_capacity(values.len());
    for (key, value) in values {
        let key = serde_json::to_string(&key.to_string()).map_err(|source| {
            CoreError::SerializeExtraction {
                path: path.clone(),
                source,
            }
        })?;
        let value =
            serde_json::to_string(value).map_err(|source| CoreError::SerializeExtraction {
                path: path.clone(),
                source,
            })?;
        fields.push(format!("{key}: {value}"));
    }
    write_text_atomic(path, &format!("{{{}}}", fields.join(", ")))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::error::Error;
    use std::fs::OpenOptions;

    use compass_model::code_graph::{
        BuildMetadata, EdgeKind, EdgeRecord, ExtractionStatus, FileRecord, NodeKind, NodeRecord,
    };
    use compass_model::identity::file_id;
    use compass_model::provenance::{EvidenceConfidence, EvidenceOrigin, Provenance, SourceAnchor};
    use serde_json::Value;
    use sha2::{Digest, Sha256};
    use tempfile::TempDir;

    use super::*;

    #[test]
    fn cluster_only_health_does_not_invent_completeness_or_freshness() {
        let health = cluster_only_orientation_health();
        assert_eq!(health.freshness, FreshnessStatus::Unknown);
        assert_eq!(health.freshness_basis, FreshnessBasis::Unavailable);
        assert_eq!(health.publication, None);
        assert_eq!(health.omitted_nodes, None);
        assert!(!health.corpus_measurements_available);
        assert_eq!(health.build_profile.as_deref(), Some("cluster-only"));
    }

    #[test]
    fn cluster_only_report_preserves_commit_identity_without_claiming_freshness() {
        let options = cluster_only_report_options("fixture", 7, Some("abc123"));
        assert_eq!(options.built_at_commit, Some("abc123"));
        assert_eq!(options.min_community_size, 7);
        assert_eq!(options.health.freshness, FreshnessStatus::Unknown);
        assert_eq!(options.health.freshness_basis, FreshnessBasis::Unavailable);
    }

    #[test]
    fn recluster_normalization_matches_python_simple_graph_edges() {
        let mut document: GraphDocument = serde_json::from_str(
            r#"{
                "nodes":[{"id":"a"},{"id":"b"}],
                "links":[
                    {"source":"a","target":"b","first":1,"shared":"old"},
                    {"source":"b","target":"a","second":2,"shared":"new"}
                ]
            }"#,
        )
        .unwrap_or_else(|_| std::process::abort());
        assert!(document.multigraph);

        normalize_recluster_document(&mut document);

        assert!(!document.multigraph);
        assert_eq!(document.links.len(), 1);
        assert_eq!(document.links[0].source, "a");
        assert_eq!(document.links[0].target, "b");
        assert_eq!(document.links[0].attributes["first"], Value::from(1));
        assert_eq!(document.links[0].attributes["second"], Value::from(2));
        assert_eq!(document.links[0].attributes["shared"], "new");
    }

    #[test]
    fn cluster_only_publishes_one_coherent_snapshot_after_an_interrupted_staging_attempt()
    -> Result<(), Box<dyn Error>> {
        let fixture = managed_graph_fixture()?;
        let previous_pointer = fs::read_to_string(fixture.output.join("current-snapshot"))?;
        let interrupted = BuildGuard::begin(&fixture.output)?;
        write_text_atomic(
            interrupted.staging_directory().join("GRAPH_REPORT.md"),
            "partial",
        )?;
        drop(interrupted);

        let result = cluster_existing_graph(&fixture.options)?;
        assert_eq!(result.nodes, 2);
        let current_pointer = fs::read_to_string(fixture.output.join("current-snapshot"))?;
        assert_ne!(current_pointer, previous_pointer);
        let current = BuildGuard::resolve_current_snapshot_directory(&fixture.output)?;
        for artifact in [
            "graph.json",
            "GRAPH_REPORT.md",
            "orientation.json",
            "analysis.json",
            "labels.json",
            "labels.json.sig",
            "graph-overview.json",
        ] {
            assert!(current.join(artifact).is_file(), "missing {artifact}");
            assert_eq!(
                fs::read(current.join(artifact))?,
                fs::read(fixture.output.join(artifact))?,
                "root projection differs for {artifact}"
            );
        }
        assert!(!current.join("graph.html").exists());
        assert!(!fixture.output.join("graph.html").exists());
        Ok(())
    }

    #[test]
    fn typed_cluster_only_publishes_graph_bound_fixed_quality_evidence()
    -> Result<(), Box<dyn Error>> {
        let fixture = managed_typed_graph_fixture()?;
        cluster_existing_graph(&fixture.options)?;
        let current = BuildGuard::resolve_current_snapshot_directory(&fixture.output)?;
        let graph_bytes = fs::read(current.join("graph.json"))?;
        let graph: V1GraphDocument = serde_json::from_slice(&graph_bytes)?;
        let quality_bytes = fs::read(current.join("community-quality.json"))?;
        assert_eq!(
            quality_bytes,
            fs::read(fixture.output.join("community-quality.json"))?
        );
        let quality: CommunityQualityArtifact = serde_json::from_slice(&quality_bytes)?;
        quality.validate_for_graph(
            &graph.graph.build.generation_id,
            &format!("sha256:{:x}", Sha256::digest(&graph_bytes)),
        )?;
        assert_eq!(
            quality.identity.algorithm,
            compass_graph::QUALITY_CLUSTER_ALGORITHM
        );
        assert_eq!(
            quality.identity.selector,
            compass_graph::COMPATIBILITY_CLUSTER_SELECTOR
        );
        assert_eq!(quality.partition.candidate_summaries.len(), 1);
        Ok(())
    }

    #[test]
    fn typed_cluster_only_publishes_a_graph_bound_hierarchy() -> Result<(), Box<dyn Error>> {
        let fixture = managed_typed_graph_fixture()?;
        cluster_existing_graph(&fixture.options)?;
        let current = BuildGuard::resolve_current_snapshot_directory(&fixture.output)?;
        let graph_bytes = fs::read(current.join("graph.json"))?;
        let graph: V1GraphDocument = serde_json::from_slice(&graph_bytes)?;
        let hierarchy_bytes = fs::read(current.join("community-hierarchy.json"))?;
        assert_eq!(
            hierarchy_bytes,
            fs::read(fixture.output.join("community-hierarchy.json"))?,
            "the root projection must carry the hierarchy"
        );
        let hierarchy: CommunityHierarchy = serde_json::from_slice(&hierarchy_bytes)?;
        hierarchy.validate_for_graph(
            &graph.graph.build.generation_id,
            &format!("sha256:{:x}", Sha256::digest(&graph_bytes)),
        )?;
        let published_communities = graph
            .nodes
            .iter()
            .filter_map(|node| node.community.as_ref().map(|community| community.id))
            .collect::<std::collections::BTreeSet<_>>()
            .len();
        assert_eq!(
            hierarchy.finest_community_count, published_communities,
            "the finest level is the published partition"
        );
        let finest = hierarchy.finest().ok_or("missing finest level")?;
        assert!(
            finest
                .groups
                .iter()
                .all(|group| group.label.evidence.contains_key("rule")),
            "every group label carries provenance"
        );

        // An unchanged rebuild republishes the same bytes rather than a new
        // hierarchy derived from a different partition.
        cluster_existing_graph(&fixture.options)?;
        let current = BuildGuard::resolve_current_snapshot_directory(&fixture.output)?;
        assert_eq!(
            fs::read(current.join("community-hierarchy.json"))?,
            hierarchy_bytes
        );
        Ok(())
    }

    #[test]
    fn cluster_only_publishes_disambiguated_community_labels_in_graph_report()
    -> Result<(), Box<dyn Error>> {
        let mut fixture = managed_graph_fixture_with_json(
            r#"{
                "directed": false,
                "multigraph": false,
                "graph": {},
                "nodes": [
                    {"id": "a", "label": "shared", "kind": "function", "source_file": "crates/core/src/left.rs", "line_start": 10},
                    {"id": "b", "label": "left_member", "kind": "function", "source_file": "crates/core/src/left.rs", "line_start": 20},
                    {"id": "c", "label": "shared", "kind": "function", "source_file": "crates/core/src/right.rs", "line_start": 30},
                    {"id": "d", "label": "right_member", "kind": "function", "source_file": "crates/core/src/right.rs", "line_start": 40}
                ],
                "links": [
                    {"source": "a", "target": "b", "relation": "calls"},
                    {"source": "c", "target": "d", "relation": "calls"}
                ]
            }"#,
        )?;
        fixture.options.no_label = false;
        fixture.options.min_community_size = 1;

        let result = cluster_existing_graph(&fixture.options)?;
        assert_eq!(result.communities, 2);
        let report = fs::read_to_string(fixture.output.join("GRAPH_REPORT.md"))?;
        assert!(report.contains("Evidence label: shared (src/left.rs:L10)"));
        assert!(report.contains("Evidence label: shared (src/right.rs:L30)"));
        Ok(())
    }

    #[test]
    fn cluster_only_failure_does_not_publish_a_partial_artifact_set() -> Result<(), Box<dyn Error>>
    {
        let fixture = managed_graph_fixture()?;
        fs::create_dir(fixture.active.join("analysis.json"))?;
        write_text_atomic(
            fixture.active.join("analysis.json").join("blocker"),
            "force the staged atomic writer to fail",
        )?;
        let pointer_before = fs::read(fixture.output.join("current-snapshot"))?;
        let graph_before = fs::read(fixture.output.join("graph.json"))?;

        assert!(cluster_existing_graph(&fixture.options).is_err());

        assert_eq!(
            fs::read(fixture.output.join("current-snapshot"))?,
            pointer_before
        );
        assert_eq!(fs::read(fixture.output.join("graph.json"))?, graph_before);
        assert_eq!(
            BuildGuard::resolve_current_snapshot_directory(&fixture.output)?,
            fixture.active
        );
        assert!(!fixture.output.join("GRAPH_REPORT.md").exists());
        assert!(!fixture.output.join("orientation.json").exists());
        Ok(())
    }

    #[test]
    fn cluster_only_rejects_an_invalid_declared_v1_graph_instead_of_publishing_legacy_json()
    -> Result<(), Box<dyn Error>> {
        let fixture = managed_graph_fixture()?;
        write_text_atomic(
            fixture.active.join("graph.json"),
            r#"{
                "directed": true,
                "multigraph": true,
                "graph": {"schema": "compass.graph/1"},
                "nodes": [{"id": "legacy-shaped-node", "label": "Legacy"}],
                "links": []
            }"#,
        )?;
        let pointer_before = fs::read(fixture.output.join("current-snapshot"))?;

        assert!(cluster_existing_graph(&fixture.options).is_err());

        assert_eq!(
            fs::read(fixture.output.join("current-snapshot"))?,
            pointer_before
        );
        assert!(!fixture.output.join("orientation.json").exists());
        Ok(())
    }

    #[test]
    fn cluster_only_rejects_an_oversized_declared_v1_graph_before_fallback()
    -> Result<(), Box<dyn Error>> {
        assert_oversized_graph_is_rejected(r#"{"graph":{"schema":"compass.graph/1"}}"#)
    }

    #[test]
    fn cluster_only_rejects_an_oversized_legacy_graph_before_loading() -> Result<(), Box<dyn Error>>
    {
        assert_oversized_graph_is_rejected(r#"{"graph":{}}"#)
    }

    fn assert_oversized_graph_is_rejected(prefix: &str) -> Result<(), Box<dyn Error>> {
        let fixture = managed_graph_fixture()?;
        write_text_atomic(fixture.active.join("graph.json"), prefix)?;
        OpenOptions::new()
            .write(true)
            .open(fixture.active.join("graph.json"))?
            .set_len(compass_model::DEFAULT_GRAPH_SIZE_CAP_BYTES + 1)?;
        let pointer_before = fs::read(fixture.output.join("current-snapshot"))?;

        assert!(cluster_existing_graph(&fixture.options).is_err());

        assert_eq!(
            fs::read(fixture.output.join("current-snapshot"))?,
            pointer_before
        );
        assert!(!fixture.output.join("orientation.json").exists());
        Ok(())
    }

    struct ManagedGraphFixture {
        _temporary: TempDir,
        output: PathBuf,
        active: PathBuf,
        options: ClusterExistingOptions,
    }

    fn managed_graph_fixture() -> Result<ManagedGraphFixture, Box<dyn Error>> {
        managed_graph_fixture_with_json(
            r#"{
                "directed": true,
                "multigraph": false,
                "graph": {},
                "nodes": [
                    {"id": "a", "label": "A", "kind": "function", "language": "rust", "file": "src/lib.rs", "line": 1},
                    {"id": "b", "label": "B", "kind": "function", "language": "rust", "file": "src/lib.rs", "line": 2}
                ],
                "links": [
                    {"source": "a", "target": "b", "relation": "calls", "file": "src/lib.rs", "line": 1}
                ]
            }"#,
        )
    }

    fn managed_typed_graph_fixture() -> Result<ManagedGraphFixture, Box<dyn Error>> {
        let digest = format!("sha256:{}", "0".repeat(64));
        let mut document = V1GraphDocument::empty_v1(BuildMetadata {
            builder_version: "test".to_owned(),
            schema_fingerprint: digest.clone(),
            source_tree_digest: digest.clone(),
            configuration_digest: digest.clone(),
            generation_id: digest.clone(),
            source_commit: None,
        });
        document.graph.files.push(FileRecord {
            id: file_id("src/lib.rs"),
            path: "src/lib.rs".to_owned(),
            language: Some("rust".to_owned()),
            content_digest: digest.clone(),
            byte_size: 2,
            generated: false,
            extraction_status: ExtractionStatus::Extracted,
            extractor_versions: vec!["cluster-existing-test".to_owned()],
            coverage: Vec::new(),
            diagnostics: Vec::new(),
        });
        let anchor = |index: u64| SourceAnchor {
            file: "src/lib.rs".to_owned(),
            start_byte: index,
            end_byte: index + 1,
            start_line: 1,
            start_column: u32::try_from(index).unwrap_or_default(),
            end_line: 1,
            end_column: u32::try_from(index + 1).unwrap_or(u32::MAX),
        };
        let evidence = |anchor: SourceAnchor| Provenance {
            origin: EvidenceOrigin::Ast,
            extractor: "cluster-existing-test".to_owned(),
            confidence: EvidenceConfidence::Exact,
            rule: None,
            anchors: vec![anchor],
            wiring_site: None,
            score: None,
            candidates: Vec::new(),
        };
        document.nodes = ["a", "b"]
            .into_iter()
            .enumerate()
            .map(|(index, id)| NodeRecord {
                id: id.to_owned(),
                kind: NodeKind::Function,
                roles: Vec::new(),
                name: id.to_owned(),
                qualified_name: id.to_owned(),
                language: Some("rust".to_owned()),
                framework: None,
                source: Some(anchor(u64::try_from(index).unwrap_or(u64::MAX - 1))),
                details: None,
                evidence: vec![evidence(anchor(
                    u64::try_from(index).unwrap_or(u64::MAX - 1),
                ))],
                coverage: Vec::new(),
                diagnostics: Vec::new(),
                community: None,
            })
            .collect();
        let relationship_site = anchor(0);
        let edge_id = compass_model::identity::edge_id(
            "a",
            EdgeKind::Calls,
            "b",
            Some(&relationship_site),
            None,
        );
        document.links.push(EdgeRecord {
            id: edge_id.clone(),
            key: edge_id,
            source: "a".to_owned(),
            target: "b".to_owned(),
            kind: EdgeKind::Calls,
            occurrence_rule: None,
            relationship_site: Some(relationship_site.clone()),
            details: None,
            evidence: vec![evidence(relationship_site)],
            weight: Some(1.0),
            context: None,
            deferred: false,
            diagnostics: Vec::new(),
        });
        managed_graph_fixture_with_json(&serde_json::to_string(&document)?)
    }

    fn managed_graph_fixture_with_json(
        graph_json: &str,
    ) -> Result<ManagedGraphFixture, Box<dyn Error>> {
        let temporary = tempfile::tempdir()?;
        let output = temporary.path().join("compass-out");
        let guard = BuildGuard::begin(&output)?;
        write_text_atomic(guard.staging_directory().join("graph.json"), graph_json)?;
        guard.commit_with_artifacts(&["graph.json"])?;
        BuildGuard::publish_root_artifacts(&output, &["graph.json"], true)?;
        let active = BuildGuard::resolve_current_snapshot_directory(&output)?;
        let options = ClusterExistingOptions {
            graph_path: active.join("graph.json"),
            output_dir: active.clone(),
            root: temporary.path().to_path_buf(),
            no_viz: true,
            no_label: true,
            resolution: 1.0,
            resolution_explicit: false,
            exclude_hubs: None,
            min_community_size: 1,
        };
        Ok(ManagedGraphFixture {
            _temporary: temporary,
            output,
            active,
            options,
        })
    }
}
