use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Component, Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use compass_files::{write_bytes_atomic, write_json_atomic, write_text_atomic};
use compass_graph::{
    Communities, community_member_signatures, god_nodes, suggest_questions, surprising_connections,
};
use compass_model::GraphDocument;
use serde_json::Value;

use crate::{
    DetectionSummary, HtmlOptions, OutputError, ReportOptions, TokenCost, TreeOptions,
    generate_report, write_html, write_tree_html,
};

pub const SUPPORTED_HISTORY_RENDERER: &str = "compass-output/v1";
static STAGING_NONCE: AtomicU64 = AtomicU64::new(0);

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DerivedArtifactRequest {
    pub relative_path: String,
    pub regeneration_version: String,
}

pub struct HistoryBundleInput<'a> {
    pub document: &'a GraphDocument,
    pub analysis: Option<&'a Value>,
    pub labels: Option<&'a Value>,
    pub manifest: Option<&'a Value>,
    pub authoritative_sidecars: &'a BTreeMap<String, Vec<u8>>,
    pub semantic_marker: &'a Value,
    pub derived: &'a [DerivedArtifactRequest],
}

pub fn publish_history_bundle(
    destination: &Path,
    input: &HistoryBundleInput<'_>,
) -> Result<(), OutputError> {
    if destination.exists() {
        return Err(OutputError::HistoryBundleExists(destination.to_path_buf()));
    }
    validate_requests(input.derived)?;
    validate_sidecars(input.authoritative_sidecars)?;
    let parent = destination.parent().unwrap_or_else(|| Path::new("."));
    fs::create_dir_all(parent).map_err(|source| io(parent, source))?;
    let file_name = destination
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| OutputError::UnsafeHistoryPath(destination.display().to_string()))?;
    let staging = unique_staging(parent, file_name)?;
    let result = build_staging(&staging, input).and_then(|()| {
        validate_staging(&staging, input)?;
        fs::rename(&staging, destination).map_err(|source| io(destination, source))
    });
    if result.is_err() && staging.exists() {
        let _cleanup = fs::remove_dir_all(&staging);
    }
    result
}

fn build_staging(staging: &Path, input: &HistoryBundleInput<'_>) -> Result<(), OutputError> {
    write_json_atomic(staging.join("graph.json"), input.document, false)?;
    if let Some(value) = input.analysis {
        write_json_atomic(staging.join(".graphify_analysis.json"), value, false)?;
    }
    if let Some(value) = input.labels {
        write_json_atomic(staging.join(".graphify_labels.json"), value, false)?;
    }
    if let Some(value) = input.manifest {
        write_json_atomic(staging.join("manifest.json"), value, false)?;
    }
    write_json_atomic(
        staging.join(".graphify_semantic_marker"),
        input.semantic_marker,
        false,
    )?;
    for (relative, bytes) in input.authoritative_sidecars {
        let destination = staging.join(relative);
        if let Some(parent) = destination.parent() {
            fs::create_dir_all(parent).map_err(|source| io(parent, source))?;
        }
        write_bytes_atomic(destination, bytes)?;
    }
    render_v1(staging, input)
}

fn render_v1(staging: &Path, input: &HistoryBundleInput<'_>) -> Result<(), OutputError> {
    let requested = input
        .derived
        .iter()
        .map(|request| request.relative_path.as_str())
        .collect::<BTreeSet<_>>();
    let communities = communities(input.document);
    let labels = labels(input.labels);
    if requested.contains("GRAPH_REPORT.md") {
        let root = input
            .document
            .graph
            .get("name")
            .and_then(Value::as_str)
            .unwrap_or("historical graph");
        let mut options = ReportOptions::new(root);
        options.built_at_commit = input
            .document
            .extras
            .get("built_at_commit")
            .and_then(Value::as_str);
        let detection = DetectionSummary {
            total_files: input
                .manifest
                .and_then(Value::as_object)
                .map_or(0, serde_json::Map::len),
            total_words: 0,
            warning: None,
        };
        let report = generate_report(
            input.document,
            &communities,
            &BTreeMap::new(),
            &labels,
            &god_nodes(input.document, 10),
            &surprising_connections(input.document, &communities, 10),
            &detection,
            TokenCost::default(),
            Some(&suggest_questions(
                input.document,
                &communities,
                &labels,
                10,
            )),
            None,
            &options,
        );
        write_text_atomic(staging.join("GRAPH_REPORT.md"), &report)?;
    }
    if requested.contains("graph.html") {
        let rendered = write_html(
            input.document,
            &communities,
            staging.join("graph.html"),
            &HtmlOptions {
                community_labels: Some(&labels),
                node_limit: Some(5_000),
                ..HtmlOptions::default()
            },
        )?;
        if rendered.is_none() {
            return Err(OutputError::InvalidHistoryBundle(
                "recorded graph.html renderer produced no output".to_owned(),
            ));
        }
    }
    if requested.contains("GRAPH_TREE.html") {
        write_tree_html(
            input.document,
            staging.join("GRAPH_TREE.html"),
            &TreeOptions::default(),
        )?;
    }
    if requested.contains(".graphify_labels.json.sig") {
        let signatures = community_member_signatures(&communities)
            .into_iter()
            .map(|(community, signature)| (community.to_string(), signature))
            .collect::<BTreeMap<_, _>>();
        write_json_atomic(
            staging.join(".graphify_labels.json.sig"),
            &signatures,
            false,
        )?;
    }
    Ok(())
}

fn validate_requests(requests: &[DerivedArtifactRequest]) -> Result<(), OutputError> {
    let mut paths = BTreeSet::new();
    for request in requests {
        safe_relative(&request.relative_path)?;
        if request.regeneration_version != SUPPORTED_HISTORY_RENDERER
            || !matches!(
                request.relative_path.as_str(),
                "GRAPH_REPORT.md" | "graph.html" | "GRAPH_TREE.html" | ".graphify_labels.json.sig"
            )
        {
            return Err(OutputError::UnsupportedHistoryRenderer {
                path: request.relative_path.clone(),
                version: request.regeneration_version.clone(),
            });
        }
        if !paths.insert(request.relative_path.as_str()) {
            return Err(OutputError::InvalidHistoryBundle(format!(
                "duplicate derived artifact {}",
                request.relative_path
            )));
        }
    }
    Ok(())
}

fn validate_staging(staging: &Path, input: &HistoryBundleInput<'_>) -> Result<(), OutputError> {
    let restored = GraphDocument::load_for_recluster_compatibility(&staging.join("graph.json"))
        .map_err(|error| OutputError::InvalidHistoryBundle(error.to_string()))?;
    if &restored != input.document {
        return Err(OutputError::InvalidHistoryBundle(
            "graph.json changed during bundle rendering".to_owned(),
        ));
    }
    for request in input.derived {
        if !staging.join(&request.relative_path).is_file() {
            return Err(OutputError::InvalidHistoryBundle(format!(
                "renderer omitted {}",
                request.relative_path
            )));
        }
    }
    for (relative, expected) in input.authoritative_sidecars {
        let actual = fs::read(staging.join(relative))
            .map_err(|source| io(staging.join(relative), source))?;
        if &actual != expected {
            return Err(OutputError::InvalidHistoryBundle(format!(
                "authoritative sidecar {relative} changed"
            )));
        }
    }
    Ok(())
}

fn communities(document: &GraphDocument) -> Communities {
    let mut communities = Communities::new();
    for node in &document.nodes {
        let community = node
            .attributes
            .get("community")
            .and_then(|value| {
                value
                    .as_u64()
                    .and_then(|value| usize::try_from(value).ok())
                    .or_else(|| value.as_str().and_then(|value| value.parse().ok()))
            })
            .unwrap_or(0);
        communities
            .entry(community)
            .or_default()
            .push(node.id.clone());
    }
    communities
}

fn labels(value: Option<&Value>) -> BTreeMap<usize, String> {
    value
        .and_then(Value::as_object)
        .into_iter()
        .flatten()
        .filter_map(|(community, label)| {
            Some((community.parse().ok()?, label.as_str()?.to_owned()))
        })
        .collect()
}

fn validate_sidecars(sidecars: &BTreeMap<String, Vec<u8>>) -> Result<(), OutputError> {
    for relative in sidecars.keys() {
        safe_relative(relative)?;
        if matches!(
            relative.as_str(),
            "graph.json"
                | "GRAPH_REPORT.md"
                | "graph.html"
                | "GRAPH_TREE.html"
                | ".graphify_analysis.json"
                | ".graphify_labels.json"
                | ".graphify_labels.json.sig"
                | "manifest.json"
                | ".graphify_semantic_marker"
        ) {
            return Err(OutputError::UnsafeHistoryPath(relative.clone()));
        }
    }
    Ok(())
}

fn safe_relative(relative: &str) -> Result<(), OutputError> {
    let path = Path::new(relative);
    if relative.is_empty()
        || path.is_absolute()
        || path.components().any(|component| {
            matches!(
                component,
                Component::ParentDir
                    | Component::RootDir
                    | Component::Prefix(_)
                    | Component::CurDir
            )
        })
    {
        Err(OutputError::UnsafeHistoryPath(relative.to_owned()))
    } else {
        Ok(())
    }
}

fn unique_staging(parent: &Path, name: &str) -> Result<PathBuf, OutputError> {
    for _ in 0..100 {
        let nonce = STAGING_NONCE.fetch_add(1, Ordering::Relaxed);
        let path = parent.join(format!(
            ".{name}.compass-history-{}-{nonce}",
            std::process::id()
        ));
        match fs::create_dir(&path) {
            Ok(()) => return Ok(path),
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
            Err(source) => return Err(io(&path, source)),
        }
    }
    Err(OutputError::InvalidHistoryBundle(
        "could not reserve a staging directory".to_owned(),
    ))
}

fn io(path: impl Into<PathBuf>, source: std::io::Error) -> OutputError {
    OutputError::HistoryBundleIo {
        path: path.into(),
        source,
    }
}
