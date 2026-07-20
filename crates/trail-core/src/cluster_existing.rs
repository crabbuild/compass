use std::collections::{BTreeMap, HashMap};
use std::fs;
use std::path::{Path, PathBuf};

use serde_json::{Value, json};
use trail_files::{write_json_atomic, write_text_atomic};
use trail_graph::{
    ClusterOptions, cluster, community_member_signatures, god_nodes, label_communities_by_hub,
    remap_communities_to_previous, score_communities, suggest_questions, surprising_connections,
};
use trail_model::GraphDocument;
use trail_output::{
    DetectionSummary, HtmlOptions, JsonExportOptions, ReportOptions, TokenCost, generate_report,
    write_html, write_json,
};

use crate::CoreError;
use crate::pipeline::{git_commit, remove_if_exists};

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
}

pub fn cluster_existing_graph(
    options: &ClusterExistingOptions,
) -> Result<ClusterExistingResult, CoreError> {
    let document = GraphDocument::load(&options.graph_path)?;
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
    let mut labels_reused = 0;
    let labels = communities
        .keys()
        .map(|community| {
            if options.no_label {
                return (*community, format!("Community {community}"));
            }
            let reusable = saved_signatures.get(community) == signatures.get(community);
            let label = if reusable {
                saved_labels.get(community).cloned().inspect(|_| {
                    labels_reused += 1;
                })
            } else {
                None
            }
            .unwrap_or_else(|| hub_labels[community].clone());
            (*community, label)
        })
        .collect::<BTreeMap<_, _>>();
    let cohesion = score_communities(&document, &communities);
    let gods = god_nodes(&document, 10);
    let surprises = surprising_connections(&document, &communities, 5);
    let questions = suggest_questions(&document, &communities, &labels, 10);
    let commit = git_commit(&options.root);
    let report_root = options.root.to_string_lossy();
    let mut report_options = ReportOptions::new(&report_root);
    report_options.min_community_size = options.min_community_size;
    report_options.built_at_commit = commit.as_deref();
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
        TokenCost::default(),
        Some(&questions),
        None,
        &report_options,
    );
    write_text_atomic(options.output_dir.join("GRAPH_REPORT.md"), &report)?;
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
    write_json_atomic(
        options.output_dir.join(".graphify_labels.json"),
        &labels,
        false,
    )?;
    write_json_atomic(
        options.output_dir.join(".graphify_labels.json.sig"),
        &signatures,
        false,
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
        labels_reused,
        html_written,
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
