use std::collections::{BTreeMap, HashMap};
use std::fs;
use std::path::Path;

use serde_json::{Map, Value};
use trail_files::write_text_atomic;
use trail_graph::Communities;
use trail_model::GraphDocument;
use unicode_normalization::{UnicodeNormalization, char::is_combining_mark};

use crate::OutputError;

const DEFAULT_GRAPH_SIZE_CAP: u64 = 512 * 1024 * 1024;

#[derive(Clone, Debug, Default)]
pub struct JsonExportOptions<'a> {
    pub force: bool,
    pub built_at_commit: Option<&'a str>,
    pub community_labels: Option<&'a BTreeMap<usize, String>>,
}

#[must_use]
pub fn export_json_value(
    document: &GraphDocument,
    communities: &Communities,
    options: &JsonExportOptions<'_>,
) -> Value {
    let node_community = communities
        .iter()
        .flat_map(|(community, members)| {
            members
                .iter()
                .map(move |member| (member.as_str(), *community))
        })
        .collect::<HashMap<_, _>>();
    let nodes = document
        .nodes
        .iter()
        .map(|node| {
            let mut output = node.attributes.clone();
            output.insert("id".to_owned(), Value::String(node.id.clone()));
            let community = node_community.get(node.id.as_str()).copied();
            output.insert(
                "community".to_owned(),
                community.map_or(Value::Null, |value| Value::from(value as u64)),
            );
            if let (Some(community), Some(labels)) = (community, options.community_labels) {
                output.insert(
                    "community_name".to_owned(),
                    Value::String(
                        labels
                            .get(&community)
                            .cloned()
                            .unwrap_or_else(|| format!("Community {community}")),
                    ),
                );
            }
            let normalized = output
                .get("label")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .nfkd()
                .filter(|character| !is_combining_mark(*character))
                .collect::<String>()
                .to_lowercase();
            output.insert("norm_label".to_owned(), Value::String(normalized));
            Value::Object(output)
        })
        .collect::<Vec<_>>();
    let links = document
        .links
        .iter()
        .map(|edge| {
            let mut output = edge.attributes.clone();
            let needs_score = !output.contains_key("confidence_score");
            let confidence = output
                .get("confidence")
                .and_then(Value::as_str)
                .unwrap_or("EXTRACTED")
                .to_owned();
            let true_source = output.remove("_src");
            let true_target = output.remove("_tgt");
            output.insert(
                "source".to_owned(),
                true_source.unwrap_or_else(|| Value::String(edge.source.clone())),
            );
            output.insert(
                "target".to_owned(),
                true_target.unwrap_or_else(|| Value::String(edge.target.clone())),
            );
            if needs_score {
                let score = match confidence.as_str() {
                    "INFERRED" => 0.5,
                    "AMBIGUOUS" => 0.2,
                    _ => 1.0,
                };
                output.insert("confidence_score".to_owned(), Value::from(score));
            }
            Value::Object(output)
        })
        .collect::<Vec<_>>();
    let hyperedges = document
        .graph
        .get("hyperedges")
        .cloned()
        .unwrap_or_else(|| Value::Array(Vec::new()));
    let mut output = Map::new();
    output.insert("directed".to_owned(), Value::Bool(document.directed));
    output.insert("multigraph".to_owned(), Value::Bool(document.multigraph));
    output.insert("graph".to_owned(), Value::Object(document.graph.clone()));
    output.insert("nodes".to_owned(), Value::Array(nodes));
    output.insert("links".to_owned(), Value::Array(links));
    output.insert("hyperedges".to_owned(), hyperedges);
    if let Some(commit) = options.built_at_commit.filter(|commit| !commit.is_empty()) {
        output.insert(
            "built_at_commit".to_owned(),
            Value::String(commit.to_owned()),
        );
    }
    Value::Object(output)
}

pub fn write_json(
    document: &GraphDocument,
    communities: &Communities,
    output_path: impl AsRef<Path>,
    options: &JsonExportOptions<'_>,
) -> Result<(), OutputError> {
    let output_path = output_path.as_ref();
    enforce_shrink_guard(output_path, document.nodes.len(), options.force)?;
    let value = export_json_value(document, communities, options);
    let encoded =
        serde_json::to_string_pretty(&value).map_err(|source| trail_files::FileError::Json {
            path: output_path.to_path_buf(),
            source,
        })?;
    write_text_atomic(output_path, &escape_non_ascii(&encoded))?;
    Ok(())
}

fn enforce_shrink_guard(path: &Path, new_count: usize, force: bool) -> Result<(), OutputError> {
    if force || !path.exists() {
        return Ok(());
    }
    let metadata = match fs::metadata(path) {
        Ok(metadata) => metadata,
        Err(_) => return Ok(()),
    };
    if metadata.len() > graph_size_cap() {
        return Ok(());
    }
    let raw = match fs::read_to_string(path) {
        Ok(raw) => raw,
        Err(_) if metadata.len() == 0 => return Ok(()),
        Err(_) => return Err(OutputError::MalformedGraph(path.to_path_buf())),
    };
    if raw.trim().is_empty() {
        return Ok(());
    }
    let value: Value =
        serde_json::from_str(&raw).map_err(|_| OutputError::MalformedGraph(path.to_path_buf()))?;
    let existing = value
        .get("nodes")
        .and_then(Value::as_array)
        .map(Vec::len)
        .ok_or_else(|| OutputError::MalformedGraph(path.to_path_buf()))?;
    if new_count < existing {
        return Err(OutputError::ShrinkRefused {
            existing,
            new: new_count,
        });
    }
    Ok(())
}

fn graph_size_cap() -> u64 {
    let Ok(raw) = std::env::var("GRAPHIFY_MAX_GRAPH_BYTES") else {
        return DEFAULT_GRAPH_SIZE_CAP;
    };
    let text = raw.trim().to_uppercase();
    if text.is_empty() {
        return DEFAULT_GRAPH_SIZE_CAP;
    }
    let (number, multiplier) = if let Some(number) = text.strip_suffix("GB") {
        (number, 1024_u64 * 1024 * 1024)
    } else if let Some(number) = text.strip_suffix("MB") {
        (number, 1024_u64 * 1024)
    } else {
        (text.as_str(), 1)
    };
    number
        .trim()
        .parse::<u64>()
        .ok()
        .and_then(|value| value.checked_mul(multiplier))
        .filter(|value| *value > 0)
        .unwrap_or(DEFAULT_GRAPH_SIZE_CAP)
}

fn escape_non_ascii(value: &str) -> String {
    let mut output = String::with_capacity(value.len());
    for character in value.chars() {
        let code = character as u32;
        if code <= 0x7f {
            output.push(character);
        } else if code <= 0xffff {
            output.push_str(&format!("\\u{code:04x}"));
        } else {
            let scalar = code - 0x1_0000;
            let high = 0xd800 + (scalar >> 10);
            let low = 0xdc00 + (scalar & 0x3ff);
            output.push_str(&format!("\\u{high:04x}\\u{low:04x}"));
        }
    }
    output
}
