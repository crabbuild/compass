use std::collections::{BTreeMap, HashMap};
use std::fs;
use std::path::{Path, PathBuf};

use serde_json::{Value, json};
use trail_files::{write_json_atomic, write_text_atomic};
use trail_graph::{
    ClusterOptions, Communities, GodNode, cluster, community_member_signatures, god_nodes,
    label_communities_by_hub, remap_communities_to_previous, score_communities, suggest_questions,
    surprising_connections,
};
use trail_model::GraphDocument;
use trail_output::{
    DetectionSummary, HtmlOptions, JsonExportOptions, ReportOptions, TokenCost,
    backup_if_protected, generate_report, write_html, write_json,
};

use crate::pipeline::{git_commit, remove_if_exists};
use crate::{CoreError, load_learning_for_report};

#[derive(Clone, Debug)]
pub struct ClusterExistingOptions {
    pub graph_path: PathBuf,
    pub output_dir: PathBuf,
    pub root: PathBuf,
    pub no_viz: bool,
    pub no_label: bool,
    pub resolution: f64,
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
    let load_warning = GraphDocument::size_cap_exceeded(&options.graph_path).map(|(size, _)| {
        format!(
            "warning: graph.json exceeds cap ({size} bytes); falling back to community-aggregation view (node_limit=5000)"
        )
    });
    let document = GraphDocument::load_for_recluster_compatibility(&options.graph_path)?;
    if document.nodes.is_empty() {
        return Err(CoreError::EmptyGraph);
    }
    fs::create_dir_all(&options.output_dir).map_err(|source| trail_files::FileError::Io {
        path: options.output_dir.clone(),
        source,
    })?;
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
    let fresh = cluster(
        &document,
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
    let hub_labels = label_communities_by_hub(&document, &communities);
    let signatures = community_member_signatures(&communities);
    let saved_labels = load_usize_string_map(&options.output_dir.join(".graphify_labels.json"));
    let saved_signatures =
        load_usize_string_map(&options.output_dir.join(".graphify_labels.json.sig"));
    let cohesion = score_communities(&document, &communities);
    let gods = god_nodes(&document, 10);
    let surprises = surprising_connections(&document, &communities, 5);
    let selection = labeler(&ClusterLabelContext {
        document: &document,
        communities: &communities,
        hub_labels: &hub_labels,
        saved_labels: &saved_labels,
        saved_signatures: &saved_signatures,
        signatures: &signatures,
        gods: &gods,
    });
    let labels = selection.labels;
    let questions = suggest_questions(&document, &communities, &labels, 10);
    let commit_root = std::env::current_dir().unwrap_or_else(|_| options.root.clone());
    let commit = git_commit(&commit_root);
    let report_root = options.root.to_string_lossy();
    let mut report_options = ReportOptions::new(&report_root);
    report_options.min_community_size = options.min_community_size;
    report_options.built_at_commit = commit.as_deref();
    let learning = load_learning_for_report(&options.output_dir.join("graph.json"));
    let report = generate_report(
        &document,
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
        learning.as_ref(),
        &report_options,
    );
    write_text_atomic(options.output_dir.join("GRAPH_REPORT.md"), &report)?;
    let backup = backup_if_protected(&options.output_dir);
    write_json_atomic(
        options.output_dir.join(".graphify_analysis.json"),
        &json!({
            "communities": communities.iter().map(|(key, value)| (key.to_string(), value)).collect::<BTreeMap<_, _>>(),
            "cohesion": cohesion.iter().map(|(key, value)| (key.to_string(), value)).collect::<BTreeMap<_, _>>(),
            "gods": gods,
            "surprises": surprises,
            "questions": questions,
        }),
        true,
    )?;
    write_json(
        &document,
        &communities,
        options.output_dir.join("graph.json"),
        &JsonExportOptions {
            force: false,
            built_at_commit: commit.as_deref(),
            community_labels: Some(&labels),
        },
    )?;
    write_python_string_map(options.output_dir.join(".graphify_labels.json"), &labels)?;
    write_python_string_map(
        options.output_dir.join(".graphify_labels.json.sig"),
        &signatures,
    )?;
    let html_path = options.output_dir.join("graph.html");
    let html_written = if options.no_viz {
        remove_if_exists(&html_path)?;
        false
    } else {
        let rendered = write_html(
            &document,
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
    Ok(ClusterExistingResult {
        nodes: document.nodes.len(),
        edges: document.links.len(),
        communities: communities.len(),
        labels_reused: selection.labels_reused,
        html_written,
        load_warning,
        backup_message: backup.message,
        backup_warning: backup.warning,
    })
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
