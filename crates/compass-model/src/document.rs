use std::collections::BTreeMap;
use std::fs::{self, File, OpenOptions};
use std::io::{BufReader, BufWriter, Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::UNIX_EPOCH;

use serde::de::{self, IgnoredAny, MapAccess, SeqAccess, Visitor};
use serde::{Deserialize, Deserializer, Serialize};
use serde_json::{Map, Value};

use crate::GraphError;

/// One node in `NetworkX` node-link form.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct NodeRecord {
    pub id: String,
    #[serde(flatten)]
    pub attributes: Map<String, Value>,
}

impl NodeRecord {
    /// Return the normalized document role when this compatibility node has
    /// document semantics. Legacy parser-shaped roles are kept here as a
    /// read-only adaptation detail so graph consumers do not duplicate syntax
    /// vocabulary or treat it as product meaning.
    #[must_use]
    pub fn document_role(&self) -> Option<&str> {
        self.attributes
            .get("document_kind")
            .and_then(Value::as_str)
            .or_else(|| {
                let qualified_name = self
                    .attributes
                    .get("qualified_name")
                    .or_else(|| self.attributes.get("qualifiedName"))
                    .and_then(Value::as_str)?;
                if qualified_name.contains("::pipe_table_cell#") {
                    Some("pipe_table_cell")
                } else if qualified_name.contains("::pipe_table_header#") {
                    Some("pipe_table_header")
                } else if qualified_name.contains("::pipe_table_row#") {
                    Some("pipe_table_row")
                } else if qualified_name.contains("::pipe_table#") {
                    Some("pipe_table")
                } else {
                    None
                }
            })
    }

    /// Return the derived document significance used by consumer profiles.
    /// Legacy records may not carry the field, so derive the same conservative
    /// fallback as the typed graph adapter.
    #[must_use]
    pub fn document_significance(&self) -> Option<crate::code_graph::DocumentSignificance> {
        let role = self.document_role()?;
        let explicit = self
            .attributes
            .get("document_significance")
            .and_then(Value::as_str);
        Some(match explicit {
            Some("container") => crate::code_graph::DocumentSignificance::Container,
            Some("scaffolding") => crate::code_graph::DocumentSignificance::Scaffolding,
            Some("content") => crate::code_graph::DocumentSignificance::Content,
            _ => match role {
                "list" | "block_quote" | "quote" | "table" => {
                    crate::code_graph::DocumentSignificance::Container
                }
                "link_reference_definition" | "footnote_definition" => {
                    crate::code_graph::DocumentSignificance::Scaffolding
                }
                _ => crate::code_graph::DocumentSignificance::Content,
            },
        })
    }

    /// Whether this node is one of the historical Markdown parser scaffolding
    /// records. In strict graph/1 projections the role is recovered from the
    /// extractor-owned qualified-name grammar rather than a new wire field.
    #[must_use]
    pub fn is_legacy_table_scaffolding(&self) -> bool {
        matches!(
            self.document_role(),
            Some("pipe_table" | "pipe_table_header" | "pipe_table_row" | "pipe_table_cell")
        )
    }

    /// Whether this node is a compact semantic table or body-row record.
    #[must_use]
    pub fn is_semantic_table_structure(&self) -> bool {
        matches!(self.document_role(), Some("table" | "table_row"))
    }

    /// Whether this node should stay out of architecture/topology summaries
    /// while remaining available for navigation and detail inspection.
    #[must_use]
    pub fn is_table_navigation_node(&self) -> bool {
        self.is_legacy_table_scaffolding() || self.is_semantic_table_structure()
    }

    #[must_use]
    pub fn string(&self, key: &str) -> String {
        self.attributes
            .get(key)
            .and_then(value_as_python_string)
            .or_else(|| match key {
                "label" => self
                    .attributes
                    .get("name")
                    .and_then(Value::as_str)
                    .map(str::to_owned),
                "source_file" => self
                    .attributes
                    .get("source")
                    .and_then(Value::as_object)
                    .and_then(|source| source.get("file"))
                    .and_then(Value::as_str)
                    .map(str::to_owned),
                "source_location" => self
                    .attributes
                    .get("source")
                    .and_then(Value::as_object)
                    .and_then(source_anchor_location),
                "community_name" => self
                    .attributes
                    .get("community")
                    .and_then(Value::as_object)
                    .and_then(|community| community.get("label"))
                    .and_then(Value::as_str)
                    .map(str::to_owned),
                "wiring_file" => evidence_anchor(&self.attributes, "wiringSite")
                    .and_then(|site| site.get("file"))
                    .and_then(Value::as_str)
                    .map(str::to_owned),
                "wiring_location" => {
                    evidence_anchor(&self.attributes, "wiringSite").and_then(source_anchor_location)
                }
                "symbol_kind" | "type" | "node_type" => self
                    .attributes
                    .get("kind")
                    .and_then(Value::as_str)
                    .map(str::to_owned),
                _ => None,
            })
            .unwrap_or_default()
    }

    #[must_use]
    pub fn label(&self) -> &str {
        self.attributes
            .get("label")
            .and_then(Value::as_str)
            .filter(|label| !label.trim().is_empty())
            .or_else(|| {
                self.attributes
                    .get("name")
                    .and_then(Value::as_str)
                    .filter(|name| !name.trim().is_empty())
            })
            .unwrap_or(&self.id)
    }

    #[must_use]
    pub fn display_label(&self) -> String {
        if let Some(value) = self
            .attributes
            .get("label")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
        {
            return value.to_owned();
        }
        if let Some(value) = self
            .attributes
            .get("name")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
        {
            return value.to_owned();
        }
        if let Some(value) = self
            .attributes
            .get("qualifiedName")
            .or_else(|| self.attributes.get("qualified_name"))
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
        {
            return value.to_owned();
        }
        if let Some(value) = self
            .attributes
            .get("signature")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
        {
            return value.to_owned();
        }
        if let Some(file) = self.source_file().filter(|file| !file.is_empty()) {
            let file = shorten_display_path(file);
            let location = self.string("source_location");
            if location.is_empty() {
                file
            } else {
                format!("{file}:{location}")
            }
        } else {
            let location = self.string("source_location");
            if location.is_empty() {
                self.id.clone()
            } else {
                location
            }
        }
    }

    #[must_use]
    pub fn source_file(&self) -> Option<&str> {
        self.attributes
            .get("source_file")
            .and_then(Value::as_str)
            .or_else(|| {
                self.attributes
                    .get("source")
                    .and_then(Value::as_object)
                    .and_then(|source| source.get("file"))
                    .and_then(Value::as_str)
            })
    }

    #[must_use]
    pub fn language_name(&self) -> Option<&str> {
        self.attributes.get("language").and_then(Value::as_str)
    }

    #[must_use]
    pub fn kind_name(&self) -> &str {
        self.attributes
            .get("symbol_kind")
            .or_else(|| self.attributes.get("type"))
            .or_else(|| self.attributes.get("kind"))
            .and_then(Value::as_str)
            .unwrap_or("symbol")
    }

    #[must_use]
    pub fn digest(&self, key: &str) -> Option<&str> {
        self.attributes.get(key).and_then(Value::as_str)
    }

    #[must_use]
    pub fn unsigned(&self, key: &str) -> Option<u64> {
        self.attributes
            .get(key)
            .and_then(|value| {
                value.as_u64().or_else(|| {
                    (key == "community")
                        .then(|| value.as_object()?.get("id")?.as_u64())
                        .flatten()
                })
            })
            .or_else(|| {
                let source = self.attributes.get("source")?.as_object()?;
                source
                    .get(match key {
                        "start_byte" => "startByte",
                        "end_byte" => "endByte",
                        "line_start" => "startLine",
                        "line_end" => "endLine",
                        _ => return None,
                    })?
                    .as_u64()
            })
    }

    #[must_use]
    pub fn property(&self, key: &str) -> Option<Value> {
        self.logical_property(key)
    }

    /// Return the logical CompassQL property for this node.
    ///
    /// The published `compass.graph/1` representation is intentionally typed
    /// and nested (`source`, `community`, and `evidence`).  CompassQL exposes
    /// stable logical aliases so callers do not need to know that wire layout.
    /// Keep this projection in the model layer and use it for direct access,
    /// property maps, and indexes alike.
    #[must_use]
    pub fn logical_property(&self, key: &str) -> Option<Value> {
        if key == "id" {
            return Some(Value::String(self.id.clone()));
        }
        if let Some(value) = self.attributes.get(key) {
            return Some(value.clone());
        }
        match key {
            "label" => self
                .attributes
                .get("name")
                .and_then(Value::as_str)
                .map(|value| Value::String(value.to_owned())),
            "qualified_name" => self
                .attributes
                .get("qualifiedName")
                .or_else(|| self.attributes.get("qualified_name"))
                .cloned(),
            "kind" | "type" | "symbol_kind" | "node_type" => self
                .attributes
                .get("kind")
                .and_then(Value::as_str)
                .map(|value| Value::String(value.to_owned())),
            "file_type" => self
                .attributes
                .get("kind")
                .and_then(Value::as_str)
                .map(|kind| Value::String(node_file_type(kind).to_owned())),
            "source_file" => self
                .attributes
                .get("source")
                .and_then(Value::as_object)
                .and_then(|source| source.get("file"))
                .and_then(Value::as_str)
                .map(|value| Value::String(value.to_owned())),
            "source_location" => self
                .attributes
                .get("source")
                .and_then(Value::as_object)
                .and_then(source_anchor_location)
                .map(Value::String),
            "line_start" | "line_end" => self
                .attributes
                .get("source")
                .and_then(Value::as_object)
                .and_then(|source| {
                    source
                        .get(if key == "line_start" {
                            "startLine"
                        } else {
                            "endLine"
                        })
                        .and_then(Value::as_u64)
                })
                .map(Value::from),
            "community_name" => self
                .attributes
                .get("community")
                .and_then(Value::as_object)
                .and_then(|community| community.get("label"))
                .and_then(Value::as_str)
                .map(|value| Value::String(value.to_owned())),
            "_origin" => first_evidence_string(&self.attributes, "origin"),
            "confidence" => effective_confidence(&self.attributes),
            _ => None,
        }
    }

    pub fn properties(&self) -> impl Iterator<Item = (&str, Value)> {
        std::iter::once(("id", Value::String(self.id.clone()))).chain(
            self.attributes
                .iter()
                .map(|(key, value)| (key.as_str(), value.clone())),
        )
    }

    pub fn logical_properties(&self) -> impl Iterator<Item = (String, Value)> {
        let mut properties = self
            .attributes
            .iter()
            .map(|(key, value)| (key.clone(), value.clone()))
            .collect::<Vec<_>>();
        properties.push(("id".to_owned(), Value::String(self.id.clone())));
        for key in [
            "label",
            "qualified_name",
            "kind",
            "type",
            "symbol_kind",
            "node_type",
            "file_type",
            "source_file",
            "source_location",
            "line_start",
            "line_end",
            "community_name",
            "_origin",
            "confidence",
        ] {
            if !properties.iter().any(|(name, _)| name == key)
                && let Some(value) = self.logical_property(key)
            {
                properties.push((key.to_owned(), value));
            }
        }
        properties.sort_by(|left, right| left.0.cmp(&right.0));
        properties.into_iter()
    }
}

/// One edge in `NetworkX` node-link form.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct EdgeRecord {
    pub source: String,
    pub target: String,
    #[serde(flatten)]
    pub attributes: Map<String, Value>,
}

impl EdgeRecord {
    #[must_use]
    pub fn string(&self, key: &str) -> String {
        self.attributes
            .get(key)
            .and_then(value_as_python_string)
            .or_else(|| match key {
                "relation" => self
                    .attributes
                    .get("kind")
                    .and_then(Value::as_str)
                    .map(str::to_owned),
                "source_file" => self
                    .attributes
                    .get("relationshipSite")
                    .and_then(Value::as_object)
                    .and_then(|site| site.get("file"))
                    .and_then(Value::as_str)
                    .map(str::to_owned),
                "source_location" => self
                    .attributes
                    .get("relationshipSite")
                    .and_then(Value::as_object)
                    .and_then(source_anchor_location),
                "_origin" => evidence_field(&self.attributes, "origin").map(str::to_owned),
                "confidence" => evidence_field(&self.attributes, "confidence").map(|confidence| {
                    match confidence {
                        "exact" => "EXTRACTED",
                        "inferred" => "INFERRED",
                        "ambiguous" => "AMBIGUOUS",
                        "unresolved" => "UNRESOLVED",
                        other => other,
                    }
                    .to_owned()
                }),
                _ => None,
            })
            .unwrap_or_default()
    }

    #[must_use]
    pub fn relation(&self) -> &str {
        self.attributes
            .get("relation")
            .or_else(|| self.attributes.get("kind"))
            .and_then(Value::as_str)
            .unwrap_or_default()
    }

    #[must_use]
    pub fn source_file(&self) -> Option<&str> {
        self.attributes
            .get("source_file")
            .and_then(Value::as_str)
            .or_else(|| {
                self.attributes
                    .get("relationshipSite")
                    .and_then(Value::as_object)
                    .and_then(|site| site.get("file"))
                    .and_then(Value::as_str)
            })
    }

    #[must_use]
    pub fn semantic_source(&self) -> &str {
        self.attributes
            .get("_src")
            .and_then(Value::as_str)
            .unwrap_or(&self.source)
    }

    #[must_use]
    pub fn semantic_target(&self) -> &str {
        self.attributes
            .get("_tgt")
            .and_then(Value::as_str)
            .unwrap_or(&self.target)
    }

    #[must_use]
    pub fn number(&self, key: &str) -> Option<f64> {
        self.attributes.get(key).and_then(Value::as_f64)
    }

    #[must_use]
    pub fn boolean(&self, key: &str) -> Option<bool> {
        self.attributes.get(key).and_then(Value::as_bool)
    }

    #[must_use]
    pub fn unsigned(&self, key: &str) -> Option<u64> {
        self.attributes
            .get(key)
            .and_then(Value::as_u64)
            .or_else(|| {
                let site = self.attributes.get("relationshipSite")?.as_object()?;
                site.get(match key {
                    "start_byte" => "startByte",
                    "end_byte" => "endByte",
                    "line_start" => "startLine",
                    "line_end" => "endLine",
                    _ => return None,
                })?
                .as_u64()
            })
    }

    #[must_use]
    pub fn property(&self, key: &str) -> Option<Value> {
        self.logical_property(key)
    }

    /// Return the logical CompassQL property for this relationship.
    #[must_use]
    pub fn logical_property(&self, key: &str) -> Option<Value> {
        match key {
            "source" | "_src" => Some(Value::String(self.source.clone())),
            "target" | "_tgt" => Some(Value::String(self.target.clone())),
            "relation" | "type" => self
                .attributes
                .get("relation")
                .or_else(|| self.attributes.get("kind"))
                .and_then(Value::as_str)
                .map(|value| Value::String(value.to_owned())),
            "source_file" => self
                .attributes
                .get("relationshipSite")
                .and_then(Value::as_object)
                .and_then(|site| site.get("file"))
                .and_then(Value::as_str)
                .map(|value| Value::String(value.to_owned())),
            "source_location" => self
                .attributes
                .get("relationshipSite")
                .and_then(Value::as_object)
                .and_then(source_anchor_location)
                .map(Value::String),
            "line_start" | "line_end" => self
                .attributes
                .get("relationshipSite")
                .and_then(Value::as_object)
                .and_then(|site| {
                    site.get(if key == "line_start" {
                        "startLine"
                    } else {
                        "endLine"
                    })
                    .and_then(Value::as_u64)
                })
                .map(Value::from),
            "_origin" => first_evidence_string(&self.attributes, "origin"),
            "confidence" => effective_confidence(&self.attributes),
            _ => self.attributes.get(key).cloned(),
        }
    }

    pub fn properties(&self) -> impl Iterator<Item = (&str, Value)> {
        [
            ("source", Value::String(self.source.clone())),
            ("target", Value::String(self.target.clone())),
        ]
        .into_iter()
        .chain(
            self.attributes
                .iter()
                .map(|(key, value)| (key.as_str(), value.clone())),
        )
    }

    pub fn logical_properties(&self) -> impl Iterator<Item = (String, Value)> {
        let mut properties = self
            .attributes
            .iter()
            .map(|(key, value)| (key.clone(), value.clone()))
            .collect::<Vec<_>>();
        properties.push(("source".to_owned(), Value::String(self.source.clone())));
        properties.push(("target".to_owned(), Value::String(self.target.clone())));
        for key in [
            "_src",
            "_tgt",
            "relation",
            "type",
            "source_file",
            "source_location",
            "line_start",
            "line_end",
            "_origin",
            "confidence",
        ] {
            if !properties.iter().any(|(name, _)| name == key)
                && let Some(value) = self.logical_property(key)
            {
                properties.push((key.to_owned(), value));
            }
        }
        properties.sort_by(|left, right| left.0.cmp(&right.0));
        properties.into_iter()
    }
}

fn node_file_type(kind: &str) -> &'static str {
    if matches!(
        kind,
        "resource" | "document" | "paper" | "image" | "concept" | "rationale"
    ) {
        return "document";
    }
    "code"
}

fn first_evidence_string(attributes: &Map<String, Value>, key: &str) -> Option<Value> {
    attributes
        .get("evidence")
        .and_then(Value::as_array)
        .and_then(|evidence| evidence.iter().find_map(Value::as_object))
        .and_then(|evidence| evidence.get(key))
        .and_then(Value::as_str)
        .map(|value| Value::String(value.to_owned()))
}

fn effective_confidence(attributes: &Map<String, Value>) -> Option<Value> {
    let mut best = attributes
        .get("confidence")
        .and_then(Value::as_str)
        .map(normalize_confidence);
    if let Some(evidence) = attributes.get("evidence").and_then(Value::as_array) {
        for value in evidence
            .iter()
            .filter_map(Value::as_object)
            .filter_map(|entry| entry.get("confidence").and_then(Value::as_str))
        {
            let candidate = normalize_confidence(value);
            if best.is_none_or(|current| confidence_rank(candidate) > confidence_rank(current)) {
                best = Some(candidate);
            }
        }
    }
    best.map(|value| Value::String(value.to_owned()))
}

fn normalize_confidence(value: &str) -> &'static str {
    match value.to_ascii_lowercase().as_str() {
        "ambiguous" => "AMBIGUOUS",
        "inferred" => "INFERRED",
        "unresolved" => "UNRESOLVED",
        _ => "EXTRACTED",
    }
}

fn confidence_rank(value: &str) -> u8 {
    match value {
        "AMBIGUOUS" => 3,
        "UNRESOLVED" => 3,
        "INFERRED" => 2,
        _ => 1,
    }
}

fn source_anchor_location(anchor: &Map<String, Value>) -> Option<String> {
    let start_line = anchor.get("startLine")?.as_u64()?;
    let exact_range = anchor
        .get("startColumn")
        .and_then(Value::as_u64)
        .zip(anchor.get("endLine").and_then(Value::as_u64))
        .zip(anchor.get("endColumn").and_then(Value::as_u64));
    Some(exact_range.map_or_else(
        || format!("L{start_line}"),
        |((start_column, end_line), end_column)| {
            format!("L{start_line}:{start_column}-L{end_line}:{end_column}")
        },
    ))
}

fn shorten_display_path(path: &str) -> String {
    if path.len() <= 40 {
        return path.to_owned();
    }
    match (Path::new(path).parent(), Path::new(path).file_name()) {
        (Some(parent), Some(name)) => {
            let parent_name = parent.file_name().and_then(|part| part.to_str());
            if let Some(parent_name) = parent_name {
                if parent_name.is_empty() {
                    path.to_owned()
                } else {
                    format!("{parent_name}/{}", name.to_string_lossy())
                }
            } else {
                path.to_owned()
            }
        }
        _ => path.to_owned(),
    }
}

fn evidence_anchor<'a>(
    attributes: &'a Map<String, Value>,
    field: &str,
) -> Option<&'a Map<String, Value>> {
    attributes
        .get("evidence")
        .and_then(Value::as_array)
        .and_then(|evidence| {
            evidence
                .iter()
                .find_map(|entry| entry.as_object()?.get(field)?.as_object())
        })
}

fn evidence_field<'a>(attributes: &'a Map<String, Value>, field: &str) -> Option<&'a str> {
    attributes
        .get("evidence")
        .and_then(Value::as_array)
        .and_then(|evidence| evidence.first())
        .and_then(Value::as_object)
        .and_then(|evidence| evidence.get(field))
        .and_then(Value::as_str)
}

/// Full node-link document, retaining unknown top-level fields.
#[derive(Clone, Debug, PartialEq)]
pub struct GraphDocument {
    pub directed: bool,
    pub multigraph: bool,
    pub graph: Map<String, Value>,
    pub nodes: Vec<NodeRecord>,
    pub links: Vec<EdgeRecord>,
    pub extras: BTreeMap<String, Value>,
}

impl GraphDocument {
    /// Load a node-link document under the extension and size guards.
    pub fn load(path: &Path) -> Result<Self, GraphError> {
        if path.extension().and_then(|part| part.to_str()) != Some("json") {
            return Err(GraphError::InvalidExtension(path.to_path_buf()));
        }
        if let Some((size, cap)) = Self::size_cap_exceeded(path) {
            return Err(GraphError::TooLarge {
                path: crate::graph::absolute_path(path),
                size,
                cap,
            });
        }
        let before = graph_signature(path);
        if let Some(signature) = before
            && let Some(document) = load_query_cache(path, signature)
            && graph_signature(path) == Some(signature)
        {
            let _ = write_affected_cache(path, signature, &document);
            return Ok(document);
        }
        let document = Self::load_for_recluster(path)?;
        if let Some(signature) = before
            && graph_signature(path) == Some(signature)
        {
            let _ = write_query_cache(path, signature, &document);
            let _ = write_affected_cache(path, signature, &document);
        }
        Ok(document)
    }

    /// Load the bounded projection used by focused graph traversal commands.
    ///
    /// The projection preserves every value read by label scoring, seed
    /// tie-breaking, traversal filtering, and text rendering while omitting
    /// unrelated publication attributes. It is disposable and keyed by the
    /// graph file signature; the JSON graph remains authoritative.
    pub fn load_for_traversal(path: &Path) -> Result<Self, GraphError> {
        if path.extension().and_then(|part| part.to_str()) != Some("json") {
            return Err(GraphError::InvalidExtension(path.to_path_buf()));
        }
        if let Some((size, cap)) = Self::size_cap_exceeded(path) {
            return Err(GraphError::TooLarge {
                path: crate::graph::absolute_path(path),
                size,
                cap,
            });
        }
        let signature = graph_signature(path);
        if let Some(signature) = signature
            && let Some(document) = load_traversal_cache(path, signature)
            && graph_signature(path) == Some(signature)
        {
            return Ok(document);
        }
        let compact = load_traversal_projection(path)?.into_cache();
        if let Some(signature) = signature
            && graph_signature(path) == Some(signature)
            && !cache_is_valid(
                &traversal_cache_path(path),
                TRAVERSAL_CACHE_MAGIC,
                signature,
            )
        {
            let _ = write_traversal_cache(path, signature, &compact);
        }
        Ok(compact.into_document())
    }

    /// Load the compact, lossless projection required by `graph affected`.
    ///
    /// The projection retains every node endpoint and edge relation while
    /// omitting attributes that cannot influence seed resolution, traversal,
    /// or rendering. Other graph commands continue to load the full document.
    pub fn load_for_affected(path: &Path) -> Result<Self, GraphError> {
        if path.extension().and_then(|part| part.to_str()) != Some("json") {
            return Err(GraphError::InvalidExtension(path.to_path_buf()));
        }
        if let Some((size, cap)) = Self::size_cap_exceeded(path) {
            return Err(GraphError::TooLarge {
                path: crate::graph::absolute_path(path),
                size,
                cap,
            });
        }
        let signature = graph_signature(path);
        if let Some(signature) = signature
            && let Some(document) = load_affected_cache(path, signature)
            && graph_signature(path) == Some(signature)
        {
            return Ok(document);
        }
        let document = Self::load(path)?;
        if let Some(signature) = signature
            && let Some(compact) = load_affected_cache(path, signature)
            && graph_signature(path) == Some(signature)
        {
            return Ok(compact);
        }
        let compact = document.compact_for_affected();
        if let Some(signature) = signature
            && graph_signature(path) == Some(signature)
        {
            let _ = write_compact_cache(path, signature, &compact);
        }
        Ok(compact)
    }

    /// Load a node-link document for re-clustering without requiring a `.json`
    /// extension. The same configured graph-size bound applies to this path.
    pub fn load_for_recluster(path: &Path) -> Result<Self, GraphError> {
        if !path.exists() {
            return Err(GraphError::NotFound(crate::graph::absolute_path(path)));
        }
        let file = File::open(path).map_err(|source| GraphError::Read {
            path: crate::graph::absolute_path(path),
            source,
        })?;
        let cap = crate::graph::graph_size_cap();
        let size = file
            .metadata()
            .map_err(|source| GraphError::Read {
                path: crate::graph::absolute_path(path),
                source,
            })?
            .len();
        if size > cap {
            return Err(GraphError::TooLarge {
                path: crate::graph::absolute_path(path),
                size,
                cap,
            });
        }
        let mut bytes = Vec::new();
        file.take(cap.saturating_add(1))
            .read_to_end(&mut bytes)
            .map_err(|source| GraphError::Read {
                path: crate::graph::absolute_path(path),
                source,
            })?;
        if bytes.len() as u64 > cap {
            return Err(GraphError::TooLarge {
                path: crate::graph::absolute_path(path),
                size: bytes.len() as u64,
                cap,
            });
        }
        serde_json::from_slice(&bytes).map_err(GraphError::Corrupt)
    }

    #[must_use]
    pub fn size_cap_exceeded(path: &Path) -> Option<(u64, u64)> {
        let size = path.metadata().ok()?.len();
        let cap = crate::graph::graph_size_cap();
        (size > cap).then_some((size, cap))
    }

    pub(crate) fn compact_for_affected(&self) -> Self {
        let nodes = self
            .nodes
            .iter()
            .map(|node| {
                let attributes = [
                    "label",
                    "name",
                    "kind",
                    "file_type",
                    "source_file",
                    "source_location",
                    "line_start",
                    "line_end",
                ]
                .into_iter()
                .filter_map(|key| {
                    node.logical_property(key)
                        .map(|value| (key.to_owned(), value))
                })
                .collect();
                NodeRecord {
                    id: node.id.clone(),
                    attributes,
                }
            })
            .collect();
        let links = self
            .links
            .iter()
            .map(|edge| {
                let attributes = [
                    "relation",
                    "source_file",
                    "source_location",
                    "line_start",
                    "line_end",
                ]
                .into_iter()
                .filter_map(|key| {
                    edge.logical_property(key)
                        .map(|value| (key.to_owned(), value))
                })
                .collect();
                EdgeRecord {
                    source: edge.source.clone(),
                    target: edge.target.clone(),
                    attributes,
                }
            })
            .collect();
        Self {
            directed: self.directed,
            multigraph: self.multigraph,
            graph: Map::new(),
            nodes,
            links,
            extras: BTreeMap::new(),
        }
    }
}

const QUERY_CACHE_MAGIC: &[u8; 8] = b"TRAILG01";
const AFFECTED_CACHE_MAGIC: &[u8; 8] = b"TRAILA02";
const TRAVERSAL_CACHE_MAGIC: &[u8; 8] = b"TRAILT04";
const QUERY_CACHE_HEADER_LEN: usize = 28;
static QUERY_CACHE_SEQUENCE: AtomicU64 = AtomicU64::new(0);

/// Streaming input for natural traversal. Unlike [`GraphDocument`], these
/// records skip unknown fields as they are read, so a first query does not
/// materialize publication evidence, diagnostics, or graph metadata that the
/// traversal engine cannot consume.
#[derive(Deserialize)]
struct TraversalRawGraphDocument {
    #[serde(default)]
    directed: bool,
    #[serde(default = "networkx_default_multigraph")]
    multigraph: bool,
    #[serde(default)]
    nodes: Vec<TraversalRawNode>,
    links: Option<Vec<TraversalRawEdge>>,
    edges: Option<Vec<TraversalRawEdge>>,
}

#[derive(Default)]
struct TraversalRawNode {
    id: String,
    label: Option<Value>,
    name: Option<Value>,
    norm_label: Option<Value>,
    qualified_name: Option<Value>,
    kind: Option<Value>,
    symbol_kind: Option<Value>,
    node_type: Option<Value>,
    file_type: Option<Value>,
    community: Option<Value>,
    community_name: Option<Value>,
    source: Option<Value>,
    source_file: Option<Value>,
    source_location: Option<Value>,
    wiring_file: Option<Value>,
    wiring_location: Option<Value>,
    evidence_wiring_site: Option<Value>,
}

#[derive(Default)]
struct TraversalRawEdge {
    source: String,
    target: String,
    relation: Option<Value>,
    kind: Option<Value>,
    confidence: Option<Value>,
    context: Option<Value>,
    source_file: Option<Value>,
    source_location: Option<Value>,
    relationship_site: Option<Value>,
    evidence_confidence: Option<Value>,
}

#[derive(Default)]
struct TraversalEvidenceItems(Vec<TraversalEvidenceItem>);

#[derive(Default)]
struct TraversalEvidenceItem {
    wiring_site: Option<Value>,
    confidence: Option<Value>,
}

impl<'de> Deserialize<'de> for TraversalRawNode {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        struct RawNodeVisitor;

        impl<'de> Visitor<'de> for RawNodeVisitor {
            type Value = TraversalRawNode;

            fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                formatter.write_str("a node object")
            }

            fn visit_map<M>(self, mut map: M) -> Result<Self::Value, M::Error>
            where
                M: MapAccess<'de>,
            {
                let mut id = None;
                let mut node = TraversalRawNode::default();
                while let Some(key) = map.next_key::<String>()? {
                    match key.as_str() {
                        "id" => id = Some(map.next_value()?),
                        "label" => node.label = Some(map.next_value()?),
                        "name" => node.name = Some(map.next_value()?),
                        "norm_label" => node.norm_label = Some(map.next_value()?),
                        "qualifiedName" | "qualified_name" => {
                            node.qualified_name = Some(map.next_value()?);
                        }
                        "kind" => node.kind = Some(map.next_value()?),
                        "symbol_kind" => node.symbol_kind = Some(map.next_value()?),
                        "type" => node.node_type = Some(map.next_value()?),
                        "file_type" => node.file_type = Some(map.next_value()?),
                        "community" => node.community = Some(map.next_value()?),
                        "community_name" => node.community_name = Some(map.next_value()?),
                        "source" => node.source = Some(map.next_value()?),
                        "source_file" => node.source_file = Some(map.next_value()?),
                        "source_location" => node.source_location = Some(map.next_value()?),
                        "wiring_file" => node.wiring_file = Some(map.next_value()?),
                        "wiring_location" => node.wiring_location = Some(map.next_value()?),
                        "evidence" => {
                            for evidence in map.next_value::<TraversalEvidenceItems>()?.0 {
                                if node.evidence_wiring_site.is_none()
                                    && evidence.wiring_site.as_ref().is_some_and(Value::is_object)
                                {
                                    node.evidence_wiring_site = evidence.wiring_site;
                                }
                            }
                        }
                        _ => {
                            let _: IgnoredAny = map.next_value()?;
                        }
                    }
                }
                node.id = id.ok_or_else(|| de::Error::missing_field("id"))?;
                Ok(node)
            }
        }

        deserializer.deserialize_map(RawNodeVisitor)
    }
}

impl<'de> Deserialize<'de> for TraversalRawEdge {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        struct RawEdgeVisitor;

        impl<'de> Visitor<'de> for RawEdgeVisitor {
            type Value = TraversalRawEdge;

            fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                formatter.write_str("an edge object")
            }

            fn visit_map<M>(self, mut map: M) -> Result<Self::Value, M::Error>
            where
                M: MapAccess<'de>,
            {
                let mut source = None;
                let mut target = None;
                let mut edge = TraversalRawEdge::default();
                while let Some(key) = map.next_key::<String>()? {
                    match key.as_str() {
                        "source" => source = Some(map.next_value()?),
                        "target" => target = Some(map.next_value()?),
                        "relation" => edge.relation = Some(map.next_value()?),
                        "kind" => edge.kind = Some(map.next_value()?),
                        "confidence" => edge.confidence = Some(map.next_value()?),
                        "context" => edge.context = Some(map.next_value()?),
                        "source_file" => edge.source_file = Some(map.next_value()?),
                        "source_location" => edge.source_location = Some(map.next_value()?),
                        "relationshipSite" => edge.relationship_site = Some(map.next_value()?),
                        "evidence" => {
                            let first_confidence = map
                                .next_value::<TraversalEvidenceItems>()?
                                .0
                                .into_iter()
                                .next()
                                .and_then(|evidence| evidence.confidence);
                            edge.evidence_confidence = first_confidence;
                        }
                        _ => {
                            let _: IgnoredAny = map.next_value()?;
                        }
                    }
                }
                edge.source = source.ok_or_else(|| de::Error::missing_field("source"))?;
                edge.target = target.ok_or_else(|| de::Error::missing_field("target"))?;
                Ok(edge)
            }
        }

        deserializer.deserialize_map(RawEdgeVisitor)
    }
}

impl<'de> Deserialize<'de> for TraversalEvidenceItems {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        struct EvidenceItemsVisitor;

        impl<'de> Visitor<'de> for EvidenceItemsVisitor {
            type Value = TraversalEvidenceItems;

            fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                formatter.write_str("an evidence array")
            }

            fn visit_seq<A>(self, mut sequence: A) -> Result<Self::Value, A::Error>
            where
                A: SeqAccess<'de>,
            {
                let mut items = Vec::new();
                while let Some(item) = sequence.next_element()? {
                    items.push(item);
                }
                Ok(TraversalEvidenceItems(items))
            }

            fn visit_map<A>(self, mut map: A) -> Result<Self::Value, A::Error>
            where
                A: MapAccess<'de>,
            {
                while map.next_key::<IgnoredAny>()?.is_some() {
                    let _: IgnoredAny = map.next_value()?;
                }
                Ok(TraversalEvidenceItems::default())
            }

            fn visit_none<E>(self) -> Result<Self::Value, E>
            where
                E: de::Error,
            {
                Ok(TraversalEvidenceItems::default())
            }

            fn visit_unit<E>(self) -> Result<Self::Value, E>
            where
                E: de::Error,
            {
                Ok(TraversalEvidenceItems::default())
            }

            fn visit_bool<E>(self, _value: bool) -> Result<Self::Value, E>
            where
                E: de::Error,
            {
                Ok(TraversalEvidenceItems::default())
            }

            fn visit_i64<E>(self, _value: i64) -> Result<Self::Value, E>
            where
                E: de::Error,
            {
                Ok(TraversalEvidenceItems::default())
            }

            fn visit_u64<E>(self, _value: u64) -> Result<Self::Value, E>
            where
                E: de::Error,
            {
                Ok(TraversalEvidenceItems::default())
            }

            fn visit_f64<E>(self, _value: f64) -> Result<Self::Value, E>
            where
                E: de::Error,
            {
                Ok(TraversalEvidenceItems::default())
            }

            fn visit_str<E>(self, _value: &str) -> Result<Self::Value, E>
            where
                E: de::Error,
            {
                Ok(TraversalEvidenceItems::default())
            }

            fn visit_string<E>(self, _value: String) -> Result<Self::Value, E>
            where
                E: de::Error,
            {
                Ok(TraversalEvidenceItems::default())
            }
        }

        deserializer.deserialize_any(EvidenceItemsVisitor)
    }
}

impl<'de> Deserialize<'de> for TraversalEvidenceItem {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        struct EvidenceItemVisitor;

        impl<'de> Visitor<'de> for EvidenceItemVisitor {
            type Value = TraversalEvidenceItem;

            fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                formatter.write_str("an evidence object")
            }

            fn visit_map<M>(self, mut map: M) -> Result<Self::Value, M::Error>
            where
                M: MapAccess<'de>,
            {
                let mut item = TraversalEvidenceItem::default();
                while let Some(key) = map.next_key::<String>()? {
                    match key.as_str() {
                        "wiringSite" => item.wiring_site = Some(map.next_value()?),
                        "confidence" => item.confidence = Some(map.next_value()?),
                        _ => {
                            let _: IgnoredAny = map.next_value()?;
                        }
                    }
                }
                Ok(item)
            }

            fn visit_unit<E>(self) -> Result<Self::Value, E>
            where
                E: de::Error,
            {
                Ok(TraversalEvidenceItem::default())
            }
        }

        deserializer.deserialize_any(EvidenceItemVisitor)
    }
}

/// Compact, positional representation for the fields consumed by natural
/// traversal. Keeping this cache as tuples avoids a map allocation and a key
/// lookup for every retained attribute while leaving the published JSON graph
/// authoritative.
#[derive(Deserialize, Serialize)]
struct TraversalCacheDocument(bool, bool, Vec<TraversalCacheNode>, Vec<TraversalCacheEdge>);

#[derive(Deserialize, Serialize)]
struct TraversalCacheNode(
    String,
    Option<Value>,
    Option<Value>,
    Option<Value>,
    Option<Value>,
    Option<Value>,
    Option<Value>,
    Option<Value>,
    Option<Value>,
    Option<Value>,
    Option<Value>,
    Option<Value>,
    Option<Value>,
    Option<Value>,
);

#[derive(Deserialize, Serialize)]
struct TraversalCacheEdge(
    String,
    String,
    Option<Value>,
    Option<Value>,
    Option<Value>,
    Option<Value>,
    Option<Value>,
);

impl TraversalRawGraphDocument {
    fn into_cache(self) -> TraversalCacheDocument {
        let links = self.links.or(self.edges).unwrap_or_default();
        TraversalCacheDocument(
            self.directed,
            self.multigraph,
            self.nodes
                .into_iter()
                .map(TraversalRawNode::into_cache)
                .collect(),
            links
                .into_iter()
                .map(TraversalRawEdge::into_cache)
                .collect(),
        )
    }
}

impl TraversalRawNode {
    fn into_cache(self) -> TraversalCacheNode {
        let Self {
            id,
            label,
            name,
            norm_label,
            qualified_name,
            kind,
            symbol_kind,
            node_type,
            file_type,
            community,
            community_name,
            source,
            source_file,
            source_location,
            wiring_file,
            wiring_location,
            evidence_wiring_site,
        } = self;
        let source_file_value = projected_string_value(
            source_file.as_ref(),
            source_anchor_field(source.as_ref(), "file"),
        );
        let source_location_value = projected_string_value(
            source_location.as_ref(),
            source
                .as_ref()
                .and_then(Value::as_object)
                .and_then(source_anchor_location),
        );
        let wiring_file_value = projected_string_value(
            wiring_file.as_ref(),
            source_anchor_field(evidence_wiring_site.as_ref(), "file"),
        );
        let wiring_location_value = projected_string_value(
            wiring_location.as_ref(),
            evidence_wiring_site
                .as_ref()
                .and_then(Value::as_object)
                .and_then(source_anchor_location),
        );
        let community_name = community
            .as_ref()
            .and_then(|value| {
                value
                    .as_object()
                    .and_then(|community| community.get("label"))
                    .and_then(Value::as_str)
                    .map(|label| Value::String(label.to_owned()))
            })
            .or(community_name);
        let community = community.filter(|value| {
            value
                .as_object()
                .and_then(|community| community.get("label"))
                .and_then(Value::as_str)
                .is_none()
        });
        let label = label
            .filter(|value| value_as_python_string(value).is_some())
            .or(name);
        TraversalCacheNode(
            id,
            label,
            norm_label,
            qualified_name,
            kind,
            symbol_kind,
            node_type,
            file_type,
            community,
            community_name,
            source_file_value,
            source_location_value,
            wiring_file_value,
            wiring_location_value,
        )
    }
}

impl TraversalRawEdge {
    fn into_cache(self) -> TraversalCacheEdge {
        let Self {
            source,
            target,
            relation,
            kind,
            confidence,
            context,
            source_file,
            source_location,
            relationship_site,
            evidence_confidence,
        } = self;
        let confidence = confidence
            .as_ref()
            .and_then(value_as_python_string)
            .map(Value::String)
            .or_else(|| {
                evidence_confidence.map(|value| match value.as_str() {
                    Some("exact") => Value::String("EXTRACTED".to_owned()),
                    Some("inferred") => Value::String("INFERRED".to_owned()),
                    Some("ambiguous") => Value::String("AMBIGUOUS".to_owned()),
                    Some("unresolved") => Value::String("UNRESOLVED".to_owned()),
                    Some(_) | None => value,
                })
            });
        TraversalCacheEdge(
            source,
            target,
            relation
                .as_ref()
                .and_then(value_as_python_string)
                .map(Value::String)
                .or_else(|| {
                    kind.as_ref()
                        .and_then(value_as_python_string)
                        .map(Value::String)
                }),
            confidence,
            context
                .as_ref()
                .and_then(value_as_python_string)
                .map(Value::String),
            projected_string_value(
                source_file.as_ref(),
                source_anchor_field(relationship_site.as_ref(), "file"),
            ),
            projected_string_value(
                source_location.as_ref(),
                relationship_site
                    .as_ref()
                    .and_then(Value::as_object)
                    .and_then(source_anchor_location),
            ),
        )
    }
}

fn projected_string_value(direct: Option<&Value>, fallback: Option<String>) -> Option<Value> {
    direct
        .and_then(value_as_python_string)
        .or(fallback)
        .and_then(nonempty_string_value)
}

fn source_anchor_field(anchor: Option<&Value>, field: &str) -> Option<String> {
    anchor
        .and_then(Value::as_object)
        .and_then(|anchor| anchor.get(field))
        .and_then(value_as_python_string)
}

impl TraversalCacheDocument {
    fn into_document(self) -> GraphDocument {
        let Self(directed, multigraph, nodes, links) = self;
        let nodes = nodes
            .into_iter()
            .map(|node| {
                let TraversalCacheNode(
                    id,
                    label,
                    norm_label,
                    qualified_name,
                    kind,
                    symbol_kind,
                    node_type,
                    file_type,
                    community,
                    community_name,
                    source_file,
                    source_location,
                    wiring_file,
                    wiring_location,
                ) = node;
                let mut attributes = Map::new();
                insert_optional_value(&mut attributes, "label", label);
                insert_optional_value(&mut attributes, "norm_label", norm_label);
                insert_optional_value(&mut attributes, "qualified_name", qualified_name);
                insert_optional_value(&mut attributes, "kind", kind);
                insert_optional_value(&mut attributes, "symbol_kind", symbol_kind);
                insert_optional_value(&mut attributes, "type", node_type);
                insert_optional_value(&mut attributes, "file_type", file_type);
                insert_optional_value(&mut attributes, "community", community);
                insert_optional_value(&mut attributes, "community_name", community_name);
                insert_optional_value(&mut attributes, "source_file", source_file);
                insert_optional_value(&mut attributes, "source_location", source_location);
                insert_optional_value(&mut attributes, "wiring_file", wiring_file);
                insert_optional_value(&mut attributes, "wiring_location", wiring_location);
                NodeRecord { id, attributes }
            })
            .collect();
        let links = links
            .into_iter()
            .map(|edge| {
                let TraversalCacheEdge(
                    source,
                    target,
                    relation,
                    confidence,
                    context,
                    source_file,
                    source_location,
                ) = edge;
                let mut attributes = Map::new();
                insert_optional_value(&mut attributes, "relation", relation);
                insert_optional_value(&mut attributes, "confidence", confidence);
                insert_optional_value(&mut attributes, "context", context);
                insert_optional_value(&mut attributes, "source_file", source_file);
                insert_optional_value(&mut attributes, "source_location", source_location);
                EdgeRecord {
                    source,
                    target,
                    attributes,
                }
            })
            .collect();
        GraphDocument {
            directed,
            multigraph,
            graph: Map::new(),
            nodes,
            links,
            extras: BTreeMap::new(),
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct GraphSignature {
    len: u64,
    modified_secs: u64,
    modified_nanos: u32,
}

fn graph_signature(path: &Path) -> Option<GraphSignature> {
    let metadata = path.metadata().ok()?;
    let modified = metadata.modified().ok()?.duration_since(UNIX_EPOCH).ok()?;
    Some(GraphSignature {
        len: metadata.len(),
        modified_secs: modified.as_secs(),
        modified_nanos: modified.subsec_nanos(),
    })
}

fn query_cache_path(graph_path: &Path) -> PathBuf {
    let file_name = graph_path
        .file_name()
        .and_then(|value| value.to_str())
        .unwrap_or("graph.json");
    graph_path
        .parent()
        .unwrap_or_else(|| Path::new("."))
        .join("cache")
        .join(format!("{file_name}.query-v1.cache"))
}

fn affected_cache_path(graph_path: &Path) -> PathBuf {
    let file_name = graph_path
        .file_name()
        .and_then(|value| value.to_str())
        .unwrap_or("graph.json");
    graph_path
        .parent()
        .unwrap_or_else(|| Path::new("."))
        .join("cache")
        .join(format!("{file_name}.affected-v1.cache"))
}

fn traversal_cache_path(graph_path: &Path) -> PathBuf {
    let file_name = graph_path
        .file_name()
        .and_then(|value| value.to_str())
        .unwrap_or("graph.json");
    graph_path
        .parent()
        .unwrap_or_else(|| Path::new("."))
        .join("cache")
        .join(format!("{file_name}.traversal-v1.cache"))
}

fn encode_cache_header(magic: &[u8; 8], signature: GraphSignature) -> [u8; QUERY_CACHE_HEADER_LEN] {
    let mut header = [0_u8; QUERY_CACHE_HEADER_LEN];
    header[..8].copy_from_slice(magic);
    header[8..16].copy_from_slice(&signature.len.to_le_bytes());
    header[16..24].copy_from_slice(&signature.modified_secs.to_le_bytes());
    header[24..28].copy_from_slice(&signature.modified_nanos.to_le_bytes());
    header
}

fn load_query_cache(path: &Path, signature: GraphSignature) -> Option<GraphDocument> {
    load_cache(&query_cache_path(path), QUERY_CACHE_MAGIC, signature)
}

fn load_affected_cache(path: &Path, signature: GraphSignature) -> Option<GraphDocument> {
    load_cache(&affected_cache_path(path), AFFECTED_CACHE_MAGIC, signature)
}

fn load_traversal_cache(path: &Path, signature: GraphSignature) -> Option<GraphDocument> {
    let cache_path = traversal_cache_path(path);
    let mut reader = BufReader::new(File::open(&cache_path).ok()?);
    let mut header = [0_u8; QUERY_CACHE_HEADER_LEN];
    reader.read_exact(&mut header).ok()?;
    if !cache_header_matches(&cache_path, TRAVERSAL_CACHE_MAGIC, signature, &header) {
        return None;
    }
    let cache: TraversalCacheDocument = rmp_serde::from_read(reader).ok()?;
    Some(cache.into_document())
}

fn load_traversal_projection(path: &Path) -> Result<TraversalRawGraphDocument, GraphError> {
    let file = File::open(path).map_err(|source| {
        if source.kind() == std::io::ErrorKind::NotFound {
            GraphError::NotFound(crate::graph::absolute_path(path))
        } else {
            GraphError::Read {
                path: crate::graph::absolute_path(path),
                source,
            }
        }
    })?;
    serde_json::from_reader(BufReader::new(file)).map_err(GraphError::Corrupt)
}

fn load_cache(
    cache_path: &Path,
    magic: &[u8; 8],
    signature: GraphSignature,
) -> Option<GraphDocument> {
    let mut reader = BufReader::new(File::open(cache_path).ok()?);
    let mut header = [0_u8; QUERY_CACHE_HEADER_LEN];
    reader.read_exact(&mut header).ok()?;
    if !cache_header_matches(cache_path, magic, signature, &header) {
        return None;
    }
    rmp_serde::from_read(reader).ok()
}

fn cache_header_matches(
    cache_path: &Path,
    magic: &[u8; 8],
    signature: GraphSignature,
    header: &[u8; QUERY_CACHE_HEADER_LEN],
) -> bool {
    let Some(cache_size) = cache_path.metadata().ok().map(|metadata| metadata.len()) else {
        return false;
    };
    let maximum = signature.len.saturating_mul(2).saturating_add(1024 * 1024);
    cache_size <= maximum && header == &encode_cache_header(magic, signature)
}

fn cache_is_valid(cache_path: &Path, magic: &[u8; 8], signature: GraphSignature) -> bool {
    let Ok(mut file) = File::open(cache_path) else {
        return false;
    };
    let mut header = [0_u8; QUERY_CACHE_HEADER_LEN];
    file.read_exact(&mut header).is_ok()
        && cache_header_matches(cache_path, magic, signature, &header)
}

fn write_query_cache(
    graph_path: &Path,
    signature: GraphSignature,
    document: &GraphDocument,
) -> std::io::Result<()> {
    write_cache(
        &query_cache_path(graph_path),
        QUERY_CACHE_MAGIC,
        signature,
        document,
    )
}

fn write_affected_cache(
    graph_path: &Path,
    signature: GraphSignature,
    document: &GraphDocument,
) -> std::io::Result<()> {
    if cache_is_valid(
        &affected_cache_path(graph_path),
        AFFECTED_CACHE_MAGIC,
        signature,
    ) {
        return Ok(());
    }
    write_compact_cache(graph_path, signature, &document.compact_for_affected())
}

fn write_traversal_cache(
    graph_path: &Path,
    signature: GraphSignature,
    document: &TraversalCacheDocument,
) -> std::io::Result<()> {
    let cache_path = traversal_cache_path(graph_path);
    let sequence = QUERY_CACHE_SEQUENCE.fetch_add(1, Ordering::Relaxed);
    let temporary = cache_path.with_extension(format!("tmp-{}-{sequence}", std::process::id()));
    let file = OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(&temporary)?;
    let result = (|| {
        let mut writer = BufWriter::new(file);
        writer.write_all(&encode_cache_header(TRAVERSAL_CACHE_MAGIC, signature))?;
        rmp_serde::encode::write(&mut writer, document).map_err(std::io::Error::other)?;
        writer.flush()?;
        drop(writer);
        #[cfg(windows)]
        if cache_path.exists() {
            fs::remove_file(&cache_path)?;
        }
        fs::rename(&temporary, cache_path)
    })();
    if result.is_err() {
        let _ = fs::remove_file(temporary);
    }
    result
}

fn write_compact_cache(
    graph_path: &Path,
    signature: GraphSignature,
    document: &GraphDocument,
) -> std::io::Result<()> {
    write_cache(
        &affected_cache_path(graph_path),
        AFFECTED_CACHE_MAGIC,
        signature,
        document,
    )
}

fn write_cache(
    cache_path: &Path,
    magic: &[u8; 8],
    signature: GraphSignature,
    document: &GraphDocument,
) -> std::io::Result<()> {
    let sequence = QUERY_CACHE_SEQUENCE.fetch_add(1, Ordering::Relaxed);
    let temporary = cache_path.with_extension(format!("tmp-{}-{sequence}", std::process::id()));
    let file = OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(&temporary)?;
    let result = (|| {
        let mut writer = BufWriter::new(file);
        writer.write_all(&encode_cache_header(magic, signature))?;
        rmp_serde::encode::write_named(&mut writer, document).map_err(std::io::Error::other)?;
        writer.flush()?;
        drop(writer);
        #[cfg(windows)]
        if cache_path.exists() {
            fs::remove_file(&cache_path)?;
        }
        fs::rename(&temporary, cache_path)
    })();
    if result.is_err() {
        let _ = fs::remove_file(temporary);
    }
    result
}

#[derive(Deserialize)]
struct RawGraphDocument {
    #[serde(default)]
    directed: bool,
    // NetworkX's node_link_graph() treats an omitted `multigraph` member as
    // true. Compass's compact graph writer relies on that legacy default, so
    // treating omission as false would collapse parallel edges and change
    // degree-sensitive traversal semantics.
    #[serde(default = "networkx_default_multigraph")]
    multigraph: bool,
    #[serde(default)]
    graph: Map<String, Value>,
    #[serde(default)]
    nodes: Vec<NodeRecord>,
    links: Option<Vec<EdgeRecord>>,
    edges: Option<Vec<EdgeRecord>>,
    #[serde(flatten)]
    extras: BTreeMap<String, Value>,
}

const fn networkx_default_multigraph() -> bool {
    true
}

impl<'de> Deserialize<'de> for GraphDocument {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let raw = RawGraphDocument::deserialize(deserializer)?;
        // Raw extraction fragments use `edges`; persisted node-link documents use
        // `links`. Both are current inputs and serialize to the single `links` form.
        let links = raw.links.or(raw.edges).unwrap_or_default();
        Ok(Self {
            directed: raw.directed,
            multigraph: raw.multigraph,
            graph: raw.graph,
            nodes: raw.nodes,
            links,
            extras: raw.extras,
        })
    }
}

impl Serialize for GraphDocument {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        use serde::ser::SerializeMap;

        let mut map = serializer.serialize_map(Some(5 + self.extras.len()))?;
        map.serialize_entry("directed", &self.directed)?;
        map.serialize_entry("multigraph", &self.multigraph)?;
        map.serialize_entry("graph", &self.graph)?;
        map.serialize_entry("nodes", &self.nodes)?;
        map.serialize_entry("links", &self.links)?;
        for (key, value) in &self.extras {
            map.serialize_entry(key, value)?;
        }
        map.end()
    }
}

fn value_as_python_string(value: &Value) -> Option<String> {
    match value {
        Value::Null => None,
        Value::String(text) => Some(text.clone()),
        Value::Bool(value) => Some(if *value { "True" } else { "False" }.to_owned()),
        Value::Number(value) => Some(value.to_string()),
        Value::Array(_) | Value::Object(_) => Some(value.to_string()),
    }
}

fn nonempty_string_value(value: String) -> Option<Value> {
    (!value.is_empty()).then_some(Value::String(value))
}

fn insert_optional_value(attributes: &mut Map<String, Value>, key: &str, value: Option<Value>) {
    if let Some(value) = value {
        attributes.insert(key.to_owned(), value);
    }
}

#[cfg(test)]
mod tests {
    use std::fs;

    use super::{
        GraphDocument, NodeRecord, affected_cache_path, query_cache_path, traversal_cache_path,
    };

    #[test]
    fn omitted_multigraph_uses_networkx_legacy_default() {
        let document: GraphDocument = serde_json::from_str(r#"{"nodes":[{"id":"a"}],"links":[]}"#)
            .unwrap_or_else(|_| std::process::abort());
        assert!(document.multigraph);
    }

    #[test]
    fn explicit_multigraph_false_remains_false() {
        let document: GraphDocument =
            serde_json::from_str(r#"{"multigraph":false,"nodes":[{"id":"a"}],"links":[]}"#)
                .unwrap_or_else(|_| std::process::abort());
        assert!(!document.multigraph);
    }

    #[test]
    fn graph_v1_markdown_table_roles_are_derived_from_qualified_identity() {
        let node: NodeRecord = serde_json::from_value(serde_json::json!({
            "id": "cell",
            "name": "Owner: compass-model",
            "qualifiedName": "Ownership::pipe_table#1::pipe_table_row#graph-1::pipe_table_cell#2"
        }))
        .unwrap_or_else(|_| std::process::abort());
        assert_eq!(node.document_role(), Some("pipe_table_cell"));
        assert!(node.is_table_navigation_node());
    }

    #[test]
    fn typed_records_project_compatibility_source_locations()
    -> Result<(), Box<dyn std::error::Error>> {
        let document: GraphDocument = serde_json::from_str(
            r#"{
                "nodes": [{
                    "id": "a",
                    "kind": "method",
                    "name": "source",
                    "source": {
                        "file": "src/lib.rs",
                        "startByte": 20,
                        "endByte": 45,
                        "startLine": 2,
                        "startColumn": 3,
                        "endLine": 4,
                        "endColumn": 5
                    },
                    "community": {"id": 7, "label": "Core"}
                }, {
                    "id": "b",
                    "kind": "function",
                    "name": "target",
                    "evidence": [{
                        "origin": "heuristic",
                        "extractor": "compass.graph.external-placeholder",
                        "confidence": "inferred",
                        "wiringSite": {
                            "file": "src/caller.rs",
                            "startLine": 11,
                            "startColumn": 7,
                            "endLine": 11,
                            "endColumn": 13
                        }
                    }]
                }],
                "links": [{
                    "id": "edge",
                    "source": "a",
                    "target": "b",
                    "kind": "calls",
                    "relationshipSite": {
                        "file": "src/lib.rs",
                        "startByte": 80,
                        "endByte": 86,
                        "startLine": 8,
                        "startColumn": 9,
                        "endLine": 8,
                        "endColumn": 15
                    }
                }]
            }"#,
        )?;

        assert_eq!(document.nodes[0].string("source_location"), "L2:3-L4:5");
        assert_eq!(document.nodes[0].unsigned("start_byte"), Some(20));
        assert_eq!(document.nodes[0].unsigned("end_byte"), Some(45));
        assert_eq!(document.nodes[0].unsigned("line_start"), Some(2));
        assert_eq!(document.nodes[0].unsigned("line_end"), Some(4));
        assert_eq!(document.nodes[0].string("community_name"), "Core");
        assert_eq!(document.nodes[1].string("wiring_file"), "src/caller.rs");
        assert_eq!(document.nodes[1].string("wiring_location"), "L11:7-L11:13");
        assert_eq!(document.links[0].string("source_location"), "L8:9-L8:15");
        assert_eq!(document.links[0].unsigned("start_byte"), Some(80));
        assert_eq!(document.links[0].unsigned("end_byte"), Some(86));
        Ok(())
    }

    #[test]
    fn query_cache_is_visible_and_invalidates_when_the_graph_changes() {
        let directory = tempfile::tempdir().unwrap_or_else(|_| std::process::abort());
        let path = directory.path().join("graph.json");
        fs::create_dir(directory.path().join("cache")).unwrap_or_else(|_| std::process::abort());
        fs::write(&path, r#"{"nodes":[{"id":"a"}],"links":[]}"#)
            .unwrap_or_else(|_| std::process::abort());

        let first = GraphDocument::load(&path).unwrap_or_else(|_| std::process::abort());
        assert_eq!(first.nodes[0].id, "a");
        assert_eq!(
            query_cache_path(&path),
            directory.path().join("cache/graph.json.query-v1.cache")
        );
        assert!(query_cache_path(&path).is_file());

        fs::write(
            &path,
            r#"{"nodes":[{"id":"changed-and-longer"}],"links":[]}"#,
        )
        .unwrap_or_else(|_| std::process::abort());
        let changed = GraphDocument::load(&path).unwrap_or_else(|_| std::process::abort());
        assert_eq!(changed.nodes[0].id, "changed-and-longer");

        fs::write(query_cache_path(&path), b"corrupt").unwrap_or_else(|_| std::process::abort());
        let recovered = GraphDocument::load(&path).unwrap_or_else(|_| std::process::abort());
        assert_eq!(recovered.nodes[0].id, "changed-and-longer");
    }

    #[test]
    fn affected_cache_retains_contract_fields_and_omits_irrelevant_payload() {
        let directory = tempfile::tempdir().unwrap_or_else(|_| std::process::abort());
        let path = directory.path().join("graph.json");
        fs::create_dir(directory.path().join("cache")).unwrap_or_else(|_| std::process::abort());
        fs::write(
            &path,
            r#"{"directed":true,"multigraph":true,"nodes":[{"id":"a","label":"A","source_file":"a.rs","source_location":"L2","large_payload":"discard"},{"id":"b","label":"B"}],"links":[{"source":"a","target":"b","relation":"custom","confidence":"EXTRACTED"}]}"#,
        )
        .unwrap_or_else(|_| std::process::abort());

        let full = GraphDocument::load(&path).unwrap_or_else(|_| std::process::abort());
        assert_eq!(
            affected_cache_path(&path),
            directory.path().join("cache/graph.json.affected-v1.cache")
        );
        assert!(affected_cache_path(&path).is_file());
        let compact =
            GraphDocument::load_for_affected(&path).unwrap_or_else(|_| std::process::abort());
        assert_eq!(compact.directed, full.directed);
        assert_eq!(compact.multigraph, full.multigraph);
        assert_eq!(compact.nodes[0].string("label"), "A");
        assert_eq!(compact.nodes[0].string("source_file"), "a.rs");
        assert_eq!(compact.nodes[0].string("source_location"), "L2");
        assert!(!compact.nodes[0].attributes.contains_key("large_payload"));
        assert_eq!(compact.links[0].string("relation"), "custom");
        assert!(!compact.links[0].attributes.contains_key("confidence"));
    }

    #[test]
    fn traversal_cache_retains_natural_query_fields_and_omits_large_payloads() {
        let directory = tempfile::tempdir().unwrap_or_else(|_| std::process::abort());
        let path = directory.path().join("graph.json");
        fs::create_dir(directory.path().join("cache")).unwrap_or_else(|_| std::process::abort());
        fs::write(
            &path,
            r#"{
                "directed":true,
                "multigraph":true,
                "nodes":[
                    {"id":"a","name":"A","qualifiedName":"pkg::A","kind":"function","norm_label":"a","source":{"file":"src/a.py","startLine":2},"community":{"id":4,"label":"Core"},"evidence":[{"wiringSite":{"file":"src/routes.py","startLine":8}}],"large_payload":"discard"},
                    {"id":"b","label":"B"}
                ],
                "links":[{"source":"a","target":"b","kind":"calls","evidence":[{"confidence":"exact"}],"relationshipSite":{"file":"src/a.py","startLine":3},"context":"call","large_payload":"discard"}]
            }"#,
        )
        .unwrap_or_else(|_| std::process::abort());

        let compact =
            GraphDocument::load_for_traversal(&path).unwrap_or_else(|_| std::process::abort());
        assert_eq!(
            traversal_cache_path(&path),
            directory.path().join("cache/graph.json.traversal-v1.cache")
        );
        assert!(traversal_cache_path(&path).is_file());
        assert_eq!(compact.nodes[0].label(), "A");
        assert_eq!(compact.nodes[0].string("qualified_name"), "pkg::A");
        assert_eq!(compact.nodes[0].string("source_file"), "src/a.py");
        assert_eq!(compact.nodes[0].string("source_location"), "L2");
        assert_eq!(compact.nodes[0].string("wiring_file"), "src/routes.py");
        assert_eq!(compact.nodes[0].string("community_name"), "Core");
        assert_eq!(compact.links[0].string("relation"), "calls");
        assert_eq!(compact.links[0].string("confidence"), "EXTRACTED");
        assert_eq!(compact.links[0].string("source_location"), "L3");
        assert!(!compact.nodes[0].attributes.contains_key("large_payload"));
        assert!(!compact.links[0].attributes.contains_key("large_payload"));

        let cached =
            GraphDocument::load_for_traversal(&path).unwrap_or_else(|_| std::process::abort());
        assert_eq!(cached.nodes[0].string("qualified_name"), "pkg::A");
    }
}
