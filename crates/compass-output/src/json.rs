use std::borrow::Cow;
use std::collections::BTreeMap;
use std::fs;
use std::path::Path;

use compass_files::write_json_ascii_atomic;
use compass_graph::Communities;
use compass_model::{DEFAULT_GRAPH_SIZE_CAP_BYTES, EdgeRecord, GraphDocument, NodeRecord};
use rayon::prelude::*;
use serde::ser::{SerializeMap, SerializeSeq};
use serde::{Serialize, Serializer};
use serde_json::{Map, Value};
use unicode_normalization::{UnicodeNormalization, char::is_combining_mark};

use ahash::AHashMap;

use crate::OutputError;

#[derive(Clone, Debug, Default)]
pub struct JsonExportOptions<'a> {
    pub force: bool,
    pub built_at_commit: Option<&'a str>,
    pub community_labels: Option<&'a BTreeMap<usize, String>>,
}

struct BorrowedGraphExport<'a> {
    document: &'a GraphDocument,
    node_community: AHashMap<&'a str, usize>,
    normalized_labels: Vec<String>,
    options: &'a JsonExportOptions<'a>,
}

impl Serialize for BorrowedGraphExport<'_> {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let commit = self
            .options
            .built_at_commit
            .filter(|commit| !commit.is_empty());
        let mut output = serializer.serialize_map(Some(6 + usize::from(commit.is_some())))?;
        output.serialize_entry("directed", &self.document.directed)?;
        output.serialize_entry("multigraph", &self.document.multigraph)?;
        output.serialize_entry("graph", &self.document.graph)?;
        output.serialize_entry(
            "nodes",
            &BorrowedNodes {
                nodes: &self.document.nodes,
                node_community: &self.node_community,
                normalized_labels: &self.normalized_labels,
                community_labels: self.options.community_labels,
            },
        )?;
        output.serialize_entry(
            "links",
            &BorrowedLinks {
                links: &self.document.links,
            },
        )?;
        let empty = Value::Array(Vec::new());
        output.serialize_entry(
            "hyperedges",
            self.document.graph.get("hyperedges").unwrap_or(&empty),
        )?;
        if let Some(commit) = commit {
            output.serialize_entry("built_at_commit", commit)?;
        }
        output.end()
    }
}

struct BorrowedNodes<'a> {
    nodes: &'a [NodeRecord],
    node_community: &'a AHashMap<&'a str, usize>,
    normalized_labels: &'a [String],
    community_labels: Option<&'a BTreeMap<usize, String>>,
}

impl Serialize for BorrowedNodes<'_> {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let mut sequence = serializer.serialize_seq(Some(self.nodes.len()))?;
        for (node, normalized_label) in self.nodes.iter().zip(self.normalized_labels) {
            sequence.serialize_element(&BorrowedNode {
                node,
                community: self.node_community.get(node.id.as_str()).copied(),
                normalized_label,
                community_labels: self.community_labels,
            })?;
        }
        sequence.end()
    }
}

struct BorrowedNode<'a> {
    node: &'a NodeRecord,
    community: Option<usize>,
    normalized_label: &'a str,
    community_labels: Option<&'a BTreeMap<usize, String>>,
}

impl Serialize for BorrowedNode<'_> {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let community_name = self.community.and_then(|community| {
            self.community_labels.map(|labels| {
                labels.get(&community).map_or_else(
                    || Cow::Owned(format!("Community {community}")),
                    |label| Cow::Borrowed(label.as_str()),
                )
            })
        });
        let mut output = serializer.serialize_map(None)?;
        let mut emitted_id = false;
        let mut emitted_community = false;
        let mut emitted_community_name = false;
        let mut emitted_norm_label = false;
        for (key, value) in self.node.properties() {
            match key {
                "id" => {
                    output.serialize_entry(key, &self.node.id)?;
                    emitted_id = true;
                }
                "community" => {
                    output.serialize_entry(key, &self.community)?;
                    emitted_community = true;
                }
                "community_name" if community_name.is_some() => {
                    output.serialize_entry(key, &community_name)?;
                    emitted_community_name = true;
                }
                "norm_label" => {
                    output.serialize_entry(key, self.normalized_label)?;
                    emitted_norm_label = true;
                }
                _ => output.serialize_entry(key, &value)?,
            }
        }
        if !emitted_id {
            output.serialize_entry("id", &self.node.id)?;
        }
        if !emitted_community {
            output.serialize_entry("community", &self.community)?;
        }
        if community_name.is_some() && !emitted_community_name {
            output.serialize_entry("community_name", &community_name)?;
        }
        if !emitted_norm_label {
            output.serialize_entry("norm_label", self.normalized_label)?;
        }
        output.end()
    }
}

struct BorrowedLinks<'a> {
    links: &'a [EdgeRecord],
}

impl Serialize for BorrowedLinks<'_> {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let mut sequence = serializer.serialize_seq(Some(self.links.len()))?;
        for edge in self.links {
            sequence.serialize_element(&BorrowedLink { edge })?;
        }
        sequence.end()
    }
}

struct BorrowedLink<'a> {
    edge: &'a EdgeRecord,
}

impl Serialize for BorrowedLink<'_> {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        // `_src`/`_tgt` are private publication overrides, not logical query
        // aliases.  Use the raw attributes here so the streaming writer has
        // the same output contract as `export_json_value`.
        let source = self.edge.attributes.get("_src").cloned();
        let target = self.edge.attributes.get("_tgt").cloned();
        let confidence_score =
            self.edge.property("confidence_score").is_none().then(|| {
                match self.edge.string("confidence").as_str() {
                    "INFERRED" => 0.5,
                    "AMBIGUOUS" => 0.2,
                    _ => 1.0,
                }
            });
        let mut output = serializer.serialize_map(None)?;
        let mut emitted_source = false;
        let mut emitted_target = false;
        for (key, value) in self.edge.properties() {
            match key {
                "_src" | "_tgt" => {}
                "source" => {
                    if let Some(source) = &source {
                        output.serialize_entry(key, source)?;
                    } else {
                        output.serialize_entry(key, &self.edge.source)?;
                    }
                    emitted_source = true;
                }
                "target" => {
                    if let Some(target) = &target {
                        output.serialize_entry(key, target)?;
                    } else {
                        output.serialize_entry(key, &self.edge.target)?;
                    }
                    emitted_target = true;
                }
                _ => output.serialize_entry(key, &value)?,
            }
        }
        if !emitted_source {
            if let Some(source) = &source {
                output.serialize_entry("source", source)?;
            } else {
                output.serialize_entry("source", &self.edge.source)?;
            }
        }
        if !emitted_target {
            if let Some(target) = &target {
                output.serialize_entry("target", target)?;
            } else {
                output.serialize_entry("target", &self.edge.target)?;
            }
        }
        if let Some(confidence_score) = confidence_score {
            output.serialize_entry("confidence_score", &confidence_score)?;
        }
        output.end()
    }
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
        .collect::<AHashMap<_, _>>();
    let nodes = document
        .nodes
        .iter()
        .map(|node| {
            let mut output = node
                .properties()
                .map(|(key, value)| (key.to_owned(), value))
                .collect::<Map<_, _>>();
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
            let mut output = edge
                .properties()
                .map(|(key, value)| (key.to_owned(), value))
                .collect::<Map<_, _>>();
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
    let node_community = communities
        .iter()
        .flat_map(|(community, members)| {
            members
                .iter()
                .map(move |member| (member.as_str(), *community))
        })
        .collect::<AHashMap<_, _>>();
    let normalized_labels = document
        .nodes
        .par_iter()
        .map(|node| {
            node.label()
                .nfkd()
                .filter(|character| !is_combining_mark(*character))
                .collect::<String>()
                .to_lowercase()
        })
        .collect();
    write_json_ascii_atomic(
        output_path,
        &BorrowedGraphExport {
            document,
            node_community,
            normalized_labels,
            options,
        },
        false,
        false,
    )?;
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
    let Ok(raw) = std::env::var("COMPASS_MAX_GRAPH_BYTES") else {
        return DEFAULT_GRAPH_SIZE_CAP_BYTES;
    };
    let text = raw.trim().to_uppercase();
    if text.is_empty() {
        return DEFAULT_GRAPH_SIZE_CAP_BYTES;
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
        .unwrap_or(DEFAULT_GRAPH_SIZE_CAP_BYTES)
}

pub(crate) fn escape_non_ascii(value: &str) -> String {
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

#[allow(dead_code)]
pub(crate) fn python_json_compact(value: &Value) -> String {
    match value {
        Value::Null => "null".to_owned(),
        Value::Bool(value) => value.to_string(),
        Value::Number(value) => value.to_string(),
        Value::String(value) => {
            let encoded = serde_json::to_string(value).unwrap_or_else(|_| "\"\"".to_owned());
            escape_non_ascii(&encoded)
        }
        Value::Array(values) => format!(
            "[{}]",
            values
                .iter()
                .map(python_json_compact)
                .collect::<Vec<_>>()
                .join(", ")
        ),
        Value::Object(map) => format!(
            "{{{}}}",
            map.iter()
                .map(|(key, value)| format!(
                    "{}: {}",
                    python_json_compact(&Value::String(key.clone())),
                    python_json_compact(value)
                ))
                .collect::<Vec<_>>()
                .join(", ")
        ),
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;
    use std::fs;

    use compass_model::{EdgeRecord, GraphDocument, NodeRecord};
    use serde_json::{Map, Value, json};

    use super::{JsonExportOptions, export_json_value, write_json};

    #[test]
    fn borrowed_writer_matches_value_export() -> Result<(), Box<dyn std::error::Error>> {
        let document = GraphDocument {
            directed: true,
            multigraph: false,
            graph: Map::from_iter([("hyperedges".to_owned(), json!([{"nodes": ["node"]}]))]),
            nodes: vec![NodeRecord {
                id: "node".to_owned(),
                attributes: Map::from_iter([
                    ("id".to_owned(), Value::String("stale".to_owned())),
                    ("label".to_owned(), Value::String("Café".to_owned())),
                    ("community".to_owned(), Value::from(99)),
                ]),
            }],
            links: vec![EdgeRecord {
                source: "visible-source".to_owned(),
                target: "visible-target".to_owned(),
                attributes: Map::from_iter([
                    ("_src".to_owned(), Value::String("true-source".to_owned())),
                    ("_tgt".to_owned(), Value::String("true-target".to_owned())),
                    (
                        "confidence".to_owned(),
                        Value::String("INFERRED".to_owned()),
                    ),
                ]),
            }],
            extras: BTreeMap::new(),
        };
        let communities = BTreeMap::from([(7, vec!["node".to_owned()])]);
        let labels = BTreeMap::from([(7, "Core".to_owned())]);
        let options = JsonExportOptions {
            force: true,
            built_at_commit: Some("abc123"),
            community_labels: Some(&labels),
        };
        let expected = export_json_value(&document, &communities, &options);
        let directory = tempfile::tempdir()?;
        let path = directory.path().join("graph.json");

        write_json(&document, &communities, &path, &options)?;

        let actual: Value = serde_json::from_slice(&fs::read(path)?)?;
        assert_eq!(actual, expected);
        Ok(())
    }
}
