use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Component, Path};

use compass_files::{write_bytes_atomic, write_json_atomic};
use compass_model::{EdgeRecord, GraphDocument, NodeRecord};
use prolly::{KeyBuilder, VersionedValue, decode_segments};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use sha2::{Digest, Sha256};

use crate::{
    ArtifactClass, ArtifactContent, ArtifactRegistryEntry, CompletionEvidence, HistoryError,
    canonical_json_bytes, edge_key, hyperedge_key, node_key,
};

const RECORD_VERSION: u64 = 1;
const ANALYSIS_SCHEMA: &[u8] = &[1];
const ANALYSIS_KIND: &[u8] = &[4];
const METADATA_SCHEMA: &[u8] = &[1];
const METADATA_KIND: &[u8] = &[5];
const MOVED_NODE_FIELDS: [&str; 3] = ["community", "community_name", "norm_label"];

/// All authoritative inputs needed to reconstruct a complete Compass output.
#[derive(Clone, Debug, PartialEq)]
pub struct GraphArtifacts {
    pub document: GraphDocument,
    pub analysis: Option<Value>,
    pub labels: Option<Value>,
    pub manifest: Option<Value>,
    pub authoritative_sidecars: BTreeMap<String, ArtifactContent>,
}

/// Builder output coupled to authoritative completion proof.
#[derive(Clone, Debug, PartialEq)]
pub struct CompletedGraphArtifacts {
    pub artifacts: GraphArtifacts,
    pub completion: CompletionEvidence,
}

/// Deterministic typed records used to construct the five Prolly trees.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct PartitionedGraph {
    pub nodes: Vec<(Vec<u8>, Vec<u8>)>,
    pub edges: Vec<(Vec<u8>, Vec<u8>)>,
    pub hyperedges: Vec<(Vec<u8>, Vec<u8>)>,
    pub analysis: Vec<(Vec<u8>, Vec<u8>)>,
    pub metadata: Vec<(Vec<u8>, Vec<u8>)>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct DocumentHeader {
    directed: bool,
    multigraph: bool,
    graph: Map<String, Value>,
    extras: BTreeMap<String, Value>,
    used_legacy_edges_key: bool,
    graph_hyperedges_present: bool,
    top_hyperedges_present: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct OrderedRecord {
    key: Vec<u8>,
    #[serde(skip_serializing_if = "Option::is_none")]
    location: Option<HyperedgeLocation>,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum HyperedgeLocation {
    Graph,
    TopLevel,
}

impl CompletedGraphArtifacts {
    /// Load the known authoritative Compass output files after a completed build.
    pub fn load(output_dir: &Path, completion: CompletionEvidence) -> Result<Self, HistoryError> {
        completion.validate()?;
        let artifacts = GraphArtifacts::load(output_dir)?;
        Ok(Self {
            artifacts,
            completion,
        })
    }

    /// Validate and partition this completed output.
    pub fn partition(&self) -> Result<PartitionedGraph, HistoryError> {
        self.artifacts.partition(&self.completion)
    }

    /// Reconstruct graph artifacts together with the stored completion proof.
    pub fn reconstruct(partitioned: &PartitionedGraph) -> Result<Self, HistoryError> {
        let artifacts = GraphArtifacts::reconstruct(partitioned)?;
        let completion = completion_from_partition(partitioned)?;
        Ok(Self {
            artifacts,
            completion,
        })
    }

    /// Export authoritative seed inputs and a normalized compatibility marker.
    pub fn write_seed(&self, output_dir: &Path) -> Result<(), HistoryError> {
        self.artifacts.write_seed(output_dir, &self.completion)
    }
}

impl GraphArtifacts {
    /// Return the complete deterministic registry for this realization content.
    pub fn artifact_registry(&self) -> Result<Vec<ArtifactRegistryEntry>, HistoryError> {
        artifact_registry(self)
    }

    /// Load the built-in authoritative Compass artifact contract.
    pub fn load(output_dir: &Path) -> Result<Self, HistoryError> {
        Self::load_with_registry(output_dir, &[])
    }

    /// Load built-in artifacts and all opaque artifacts declared authoritative.
    pub fn load_with_registry(
        output_dir: &Path,
        registry: &[ArtifactRegistryEntry],
    ) -> Result<Self, HistoryError> {
        validate_registry_declarations(registry)?;
        let mut authoritative_sidecars = BTreeMap::new();
        for entry in registry {
            if entry.class != ArtifactClass::Authoritative
                || is_builtin_artifact(&entry.relative_path)
            {
                continue;
            }
            let bytes = fs::read(output_dir.join(&entry.relative_path)).map_err(|source| {
                crate::error::io_error(output_dir.join(&entry.relative_path), source)
            })?;
            verify_registry_content(entry, &bytes)?;
            authoritative_sidecars.insert(entry.relative_path.clone(), bytes);
        }
        let artifacts = Self {
            document: GraphDocument::load_for_recluster_compatibility(
                &output_dir.join("graph.json"),
            )?,
            analysis: read_optional_json(&output_dir.join(".graphify_analysis.json"))?,
            labels: read_optional_json(&output_dir.join(".graphify_labels.json"))?,
            manifest: read_optional_json(&output_dir.join("manifest.json"))?,
            authoritative_sidecars,
        };
        verify_builtin_registry_content(&artifacts, registry)?;
        Ok(artifacts)
    }

    /// Decompose all realization state into deterministic typed records.
    pub fn partition(
        &self,
        completion: &CompletionEvidence,
    ) -> Result<PartitionedGraph, HistoryError> {
        completion.validate()?;
        validate_sidecar_paths(&self.authoritative_sidecars)?;
        let mut partitioned = PartitionedGraph::default();
        let mut node_keys = BTreeSet::new();
        let mut edge_keys = BTreeSet::new();
        let mut hyperedge_keys = BTreeSet::new();

        for (rank, node) in self.document.nodes.iter().enumerate() {
            let mut stored = node.clone();
            for field in MOVED_NODE_FIELDS {
                if let Some(value) = stored.attributes.remove(field) {
                    partitioned.analysis.push((
                        analysis_key(&[b"node", node.id.as_bytes(), field.as_bytes()]),
                        encode_record("compass.analysis.node", &value)?,
                    ));
                }
            }
            let key = node_key(&node.id);
            if !node_keys.insert(key.clone()) {
                return Err(HistoryError::InvalidArtifacts(format!(
                    "duplicate node ID {}",
                    node.id
                )));
            }
            partitioned.nodes.push((
                key.clone(),
                encode_record("compass.node", &serde_json::to_value(stored)?)?,
            ));
            partitioned.metadata.push((
                metadata_rank_key("node-order", rank)?,
                encode_record(
                    "compass.metadata.order",
                    &serde_json::to_value(OrderedRecord {
                        key,
                        location: None,
                    })?,
                )?,
            ));
        }

        let mut edge_occurrences = BTreeMap::<Vec<u8>, u64>::new();
        for (rank, edge) in self.document.links.iter().enumerate() {
            let canonical = canonical_json_bytes(&serde_json::to_value(edge)?)?;
            let discriminator = edge_discriminator(
                edge,
                self.document.multigraph,
                &canonical,
                &mut edge_occurrences,
            )?;
            let key = edge_key(
                &edge.source,
                &edge.target,
                &edge.string("relation"),
                self.document.directed,
                discriminator.as_deref(),
            );
            if !edge_keys.insert(key.clone()) {
                return Err(HistoryError::InvalidArtifacts(format!(
                    "duplicate non-multigraph edge {} -> {}",
                    edge.source, edge.target
                )));
            }
            partitioned.edges.push((
                key.clone(),
                encode_record("compass.edge", &serde_json::to_value(edge)?)?,
            ));
            partitioned.metadata.push((
                metadata_rank_key("edge-order", rank)?,
                encode_record(
                    "compass.metadata.order",
                    &serde_json::to_value(OrderedRecord {
                        key,
                        location: None,
                    })?,
                )?,
            ));
        }

        let graph_hyperedges = hyperedge_array(self.document.graph.get("hyperedges"))?;
        let top_hyperedges = hyperedge_array(self.document.extras.get("hyperedges"))?;
        let mut hyperedge_occurrences = BTreeMap::<Vec<u8>, u64>::new();
        let mut explicit_hyperedges = BTreeSet::<Vec<u8>>::new();
        for (rank, (location, hyperedge)) in graph_hyperedges
            .iter()
            .map(|value| (HyperedgeLocation::Graph, value))
            .chain(
                top_hyperedges
                    .iter()
                    .map(|value| (HyperedgeLocation::TopLevel, value)),
            )
            .enumerate()
        {
            let canonical = canonical_json_bytes(hyperedge)?;
            let (identity, occurrence) = if let Some(id) = hyperedge.get("id") {
                let mut identity = vec![1];
                identity.extend(canonical_json_bytes(id)?);
                if !explicit_hyperedges.insert(identity.clone()) {
                    return Err(HistoryError::InvalidArtifacts(
                        "duplicate explicit hyperedge ID".to_owned(),
                    ));
                }
                (identity, None)
            } else {
                let mut identity = vec![2];
                identity.extend(Sha256::digest(&canonical));
                let occurrence = hyperedge_occurrences.entry(canonical).or_default();
                let rank = *occurrence;
                *occurrence = occurrence.saturating_add(1);
                (identity, Some(rank))
            };
            let key = hyperedge_key(&identity, occurrence);
            if !hyperedge_keys.insert(key.clone()) {
                return Err(HistoryError::InvalidArtifacts(
                    "duplicate hyperedge key".to_owned(),
                ));
            }
            partitioned
                .hyperedges
                .push((key.clone(), encode_record("compass.hyperedge", hyperedge)?));
            partitioned.metadata.push((
                metadata_rank_key("hyperedge-order", rank)?,
                encode_record(
                    "compass.metadata.order",
                    &serde_json::to_value(OrderedRecord {
                        key,
                        location: Some(location),
                    })?,
                )?,
            ));
        }

        let mut graph = self.document.graph.clone();
        let graph_hyperedges_present = graph.remove("hyperedges").is_some();
        let mut extras = self.document.extras.clone();
        let top_hyperedges_present = extras.remove("hyperedges").is_some();
        partitioned.metadata.push((
            metadata_key(&[b"document"]),
            encode_record(
                "compass.metadata.document",
                &serde_json::to_value(DocumentHeader {
                    directed: self.document.directed,
                    multigraph: self.document.multigraph,
                    graph,
                    extras,
                    used_legacy_edges_key: self.document.used_legacy_edges_key,
                    graph_hyperedges_present,
                    top_hyperedges_present,
                })?,
            )?,
        ));
        partitioned.metadata.push((
            metadata_key(&[b"completion"]),
            encode_record(
                "compass.metadata.completion",
                &serde_json::to_value(completion)?,
            )?,
        ));

        add_optional_analysis(
            &mut partitioned,
            ".graphify_analysis.json",
            self.analysis.as_ref(),
        )?;
        add_optional_analysis(
            &mut partitioned,
            ".graphify_labels.json",
            self.labels.as_ref(),
        )?;
        if let Some(manifest) = &self.manifest {
            partitioned.metadata.push((
                metadata_key(&[b"manifest"]),
                encode_record("compass.metadata.manifest", manifest)?,
            ));
        }
        for (path, bytes) in &self.authoritative_sidecars {
            partitioned.metadata.push((
                metadata_key(&[b"sidecar", path.as_bytes()]),
                encode_record("compass.metadata.sidecar", &serde_json::to_value(bytes)?)?,
            ));
        }
        let registry = artifact_registry(self)?;
        partitioned.metadata.push((
            metadata_key(&[b"artifact-registry"]),
            encode_record(
                "compass.metadata.artifact-registry",
                &serde_json::to_value(registry)?,
            )?,
        ));

        sort_unique(&mut partitioned.nodes, "node")?;
        sort_unique(&mut partitioned.edges, "edge")?;
        sort_unique(&mut partitioned.hyperedges, "hyperedge")?;
        sort_unique(&mut partitioned.analysis, "analysis")?;
        sort_unique(&mut partitioned.metadata, "metadata")?;
        Ok(partitioned)
    }

    /// Reconstruct the exact supported graph structure and authoritative sidecars.
    pub fn reconstruct(partitioned: &PartitionedGraph) -> Result<Self, HistoryError> {
        let mut nodes = decode_map::<NodeRecord>(&partitioned.nodes, "compass.node")?;
        let mut edges = decode_map::<EdgeRecord>(&partitioned.edges, "compass.edge")?;
        let mut hyperedges = decode_value_map(&partitioned.hyperedges, "compass.hyperedge")?;
        let mut node_analysis = BTreeMap::<String, Map<String, Value>>::new();
        let mut analysis = None;
        let mut labels = None;
        for (key, bytes) in &partitioned.analysis {
            let segments = decode_segments(key)
                .map_err(|error| HistoryError::InvalidArtifacts(error.to_string()))?;
            match segments.as_slice() {
                [_, _, kind, node, field] if kind == b"node" => {
                    let node = String::from_utf8(node.clone()).map_err(|error| {
                        HistoryError::InvalidArtifacts(format!("non-UTF-8 node key: {error}"))
                    })?;
                    let field = String::from_utf8(field.clone()).map_err(|error| {
                        HistoryError::InvalidArtifacts(format!("non-UTF-8 analysis key: {error}"))
                    })?;
                    node_analysis
                        .entry(node)
                        .or_default()
                        .insert(field, decode_record(bytes, "compass.analysis.node")?);
                }
                [_, _, kind, path] if kind == b"sidecar" => {
                    let value = decode_record(bytes, "compass.analysis.sidecar")?;
                    match path.as_slice() {
                        b".graphify_analysis.json" => analysis = Some(value),
                        b".graphify_labels.json" => labels = Some(value),
                        _ => {
                            return Err(HistoryError::InvalidArtifacts(
                                "unknown analysis sidecar".to_owned(),
                            ));
                        }
                    }
                }
                _ => {
                    return Err(HistoryError::InvalidArtifacts(
                        "invalid analysis key".to_owned(),
                    ));
                }
            }
        }
        for node in nodes.values_mut() {
            if let Some(fields) = node_analysis.remove(&node.id) {
                node.attributes.extend(fields);
            }
        }
        if !node_analysis.is_empty() {
            return Err(HistoryError::InvalidArtifacts(
                "analysis references a missing node".to_owned(),
            ));
        }

        let mut header = None;
        let mut completion = None;
        let mut registry = None;
        let mut manifest = None;
        let mut sidecars = BTreeMap::new();
        let mut node_order = BTreeMap::new();
        let mut edge_order = BTreeMap::new();
        let mut hyperedge_order = BTreeMap::new();
        for (key, bytes) in &partitioned.metadata {
            let segments = decode_segments(key)
                .map_err(|error| HistoryError::InvalidArtifacts(error.to_string()))?;
            match segments.as_slice() {
                [_, _, name] if name == b"document" => {
                    header = Some(decode_typed(bytes, "compass.metadata.document")?);
                }
                [_, _, name] if name == b"manifest" => {
                    manifest = Some(decode_record(bytes, "compass.metadata.manifest")?);
                }
                [_, _, name] if name == b"completion" => {
                    let evidence: CompletionEvidence =
                        decode_typed(bytes, "compass.metadata.completion")?;
                    evidence.validate()?;
                    completion = Some(evidence);
                }
                [_, _, name] if name == b"artifact-registry" => {
                    registry = Some(decode_typed::<Vec<ArtifactRegistryEntry>>(
                        bytes,
                        "compass.metadata.artifact-registry",
                    )?);
                }
                [_, _, name, path] if name == b"sidecar" => {
                    let path = String::from_utf8(path.clone()).map_err(|error| {
                        HistoryError::InvalidArtifacts(format!("non-UTF-8 sidecar path: {error}"))
                    })?;
                    let bytes: Vec<u8> = decode_typed(bytes, "compass.metadata.sidecar")?;
                    sidecars.insert(path, bytes);
                }
                [_, _, name, rank] if name == b"node-order" => {
                    node_order.insert(
                        rank_bytes(rank)?,
                        decode_typed(bytes, "compass.metadata.order")?,
                    );
                }
                [_, _, name, rank] if name == b"edge-order" => {
                    edge_order.insert(
                        rank_bytes(rank)?,
                        decode_typed(bytes, "compass.metadata.order")?,
                    );
                }
                [_, _, name, rank] if name == b"hyperedge-order" => {
                    hyperedge_order.insert(
                        rank_bytes(rank)?,
                        decode_typed(bytes, "compass.metadata.order")?,
                    );
                }
                _ => {
                    return Err(HistoryError::InvalidArtifacts(
                        "invalid metadata key".to_owned(),
                    ));
                }
            }
        }
        validate_sidecar_paths(&sidecars)?;
        let mut header: DocumentHeader = header.ok_or_else(|| {
            HistoryError::InvalidArtifacts("missing document metadata".to_owned())
        })?;
        let ordered_nodes = restore_order(&mut nodes, node_order, "node")?;
        let ordered_edges = restore_order(&mut edges, edge_order, "edge")?;
        let ordered_hyperedges = restore_hyperedge_order(&mut hyperedges, hyperedge_order)?;
        let mut graph_values = Vec::new();
        let mut top_values = Vec::new();
        for (location, value) in ordered_hyperedges {
            match location {
                HyperedgeLocation::Graph => graph_values.push(value),
                HyperedgeLocation::TopLevel => top_values.push(value),
            }
        }
        if header.graph_hyperedges_present {
            header
                .graph
                .insert("hyperedges".to_owned(), Value::Array(graph_values));
        }
        if header.top_hyperedges_present {
            header
                .extras
                .insert("hyperedges".to_owned(), Value::Array(top_values));
        }
        let restored = Self {
            document: GraphDocument {
                directed: header.directed,
                multigraph: header.multigraph,
                graph: header.graph,
                nodes: ordered_nodes,
                links: ordered_edges,
                extras: header.extras,
                used_legacy_edges_key: header.used_legacy_edges_key,
            },
            analysis,
            labels,
            manifest,
            authoritative_sidecars: sidecars,
        };
        let completion = completion.ok_or_else(|| {
            HistoryError::InvalidArtifacts("missing completion evidence".to_owned())
        })?;
        let registry = registry.ok_or_else(|| {
            HistoryError::InvalidArtifacts("missing artifact registry".to_owned())
        })?;
        if registry != artifact_registry(&restored)? {
            return Err(HistoryError::InvalidArtifacts(
                "artifact registry does not match realization content".to_owned(),
            ));
        }
        if restored.partition(&completion)? != *partitioned {
            return Err(HistoryError::InvalidArtifacts(
                "realization records are not canonical or contain invalid typed keys".to_owned(),
            ));
        }
        Ok(restored)
    }

    /// Write compatible authoritative seed inputs and normalized completion evidence.
    pub fn write_seed(
        &self,
        output_dir: &Path,
        completion: &CompletionEvidence,
    ) -> Result<(), HistoryError> {
        completion.validate()?;
        fs::create_dir_all(output_dir)
            .map_err(|source| crate::error::io_error(output_dir, source))?;
        validate_sidecar_paths(&self.authoritative_sidecars)?;
        write_json_atomic(output_dir.join("graph.json"), &self.document, false)?;
        if let Some(value) = &self.analysis {
            write_json_atomic(output_dir.join(".graphify_analysis.json"), value, false)?;
        }
        if let Some(value) = &self.labels {
            write_json_atomic(output_dir.join(".graphify_labels.json"), value, false)?;
        }
        if let Some(value) = &self.manifest {
            write_json_atomic(output_dir.join("manifest.json"), value, false)?;
        }
        for (path, bytes) in &self.authoritative_sidecars {
            let destination = output_dir.join(path);
            if let Some(parent) = destination.parent() {
                fs::create_dir_all(parent)
                    .map_err(|source| crate::error::io_error(parent, source))?;
            }
            write_bytes_atomic(destination, bytes)?;
        }
        write_json_atomic(
            output_dir.join(".graphify_semantic_marker"),
            &SemanticCompletionMarker::from(completion),
            false,
        )?;
        Ok(())
    }
}

#[derive(Serialize)]
struct SemanticCompletionMarker {
    schema: &'static str,
    schema_version: u32,
    extraction_succeeded: bool,
    allow_partial: bool,
    semantic_files_expected: u64,
    semantic_files_completed: u64,
    failed_chunks: u64,
}

impl From<&CompletionEvidence> for SemanticCompletionMarker {
    fn from(evidence: &CompletionEvidence) -> Self {
        Self {
            schema: "compass.history.completion",
            schema_version: 1,
            extraction_succeeded: evidence.extraction_succeeded,
            allow_partial: evidence.allow_partial,
            semantic_files_expected: evidence.semantic_files_expected,
            semantic_files_completed: evidence.semantic_files_completed,
            failed_chunks: evidence.failed_chunks,
        }
    }
}

fn edge_discriminator(
    edge: &EdgeRecord,
    multigraph: bool,
    canonical: &[u8],
    occurrences: &mut BTreeMap<Vec<u8>, u64>,
) -> Result<Option<Vec<u8>>, HistoryError> {
    if !multigraph {
        return Ok(None);
    }
    if let Some(key) = edge.attributes.get("key") {
        let mut discriminator = vec![1];
        discriminator.extend(canonical_json_bytes(key)?);
        return Ok(Some(discriminator));
    }
    let occurrence = occurrences.entry(canonical.to_vec()).or_default();
    let rank = *occurrence;
    *occurrence = occurrence.saturating_add(1);
    let mut discriminator = vec![2];
    discriminator.extend(Sha256::digest(canonical));
    discriminator.extend(rank.to_be_bytes());
    Ok(Some(discriminator))
}

fn add_optional_analysis(
    partitioned: &mut PartitionedGraph,
    path: &str,
    value: Option<&Value>,
) -> Result<(), HistoryError> {
    if let Some(value) = value {
        partitioned.analysis.push((
            analysis_key(&[b"sidecar", path.as_bytes()]),
            encode_record("compass.analysis.sidecar", value)?,
        ));
    }
    Ok(())
}

fn artifact_registry(
    artifacts: &GraphArtifacts,
) -> Result<Vec<ArtifactRegistryEntry>, HistoryError> {
    let graph_bytes = canonical_json_bytes(&serde_json::to_value(&artifacts.document)?)?;
    let mut registry = vec![authoritative_entry(
        "graph.json",
        "application/json",
        &graph_bytes,
    )];
    for (path, value) in [
        (".graphify_analysis.json", artifacts.analysis.as_ref()),
        (".graphify_labels.json", artifacts.labels.as_ref()),
        ("manifest.json", artifacts.manifest.as_ref()),
    ] {
        if let Some(value) = value {
            registry.push(authoritative_entry(
                path,
                "application/json",
                &canonical_json_bytes(value)?,
            ));
        }
    }
    for (path, bytes) in &artifacts.authoritative_sidecars {
        let mut entry = authoritative_entry(path, "application/octet-stream", bytes);
        entry.storage = Some(bytes.clone());
        registry.push(entry);
    }
    for path in [
        "GRAPH_REPORT.md",
        "graph.html",
        "GRAPH_TREE.html",
        ".graphify_labels.json.sig",
    ] {
        registry.push(ArtifactRegistryEntry {
            registry_version: 1,
            relative_path: path.to_owned(),
            class: ArtifactClass::Derived,
            media_type: if path.ends_with(".md") {
                "text/markdown"
            } else if path.ends_with(".json.sig") {
                "application/octet-stream"
            } else {
                "text/html"
            }
            .to_owned(),
            schema_version: None,
            content_digest: None,
            storage: None,
            regeneration_version: Some("compass-output/v1".to_owned()),
        });
    }
    registry.push(ArtifactRegistryEntry {
        registry_version: 1,
        relative_path: ".graphify_semantic_marker".to_owned(),
        class: ArtifactClass::Operational,
        media_type: "application/json".to_owned(),
        schema_version: None,
        content_digest: None,
        storage: None,
        regeneration_version: None,
    });
    registry.sort_by(|left, right| left.relative_path.cmp(&right.relative_path));
    Ok(registry)
}

fn authoritative_entry(path: &str, media_type: &str, bytes: &[u8]) -> ArtifactRegistryEntry {
    ArtifactRegistryEntry {
        registry_version: 1,
        relative_path: path.to_owned(),
        class: ArtifactClass::Authoritative,
        media_type: media_type.to_owned(),
        schema_version: Some(1),
        content_digest: Some(Sha256::digest(bytes).into()),
        storage: None,
        regeneration_version: None,
    }
}

fn completion_from_partition(
    partitioned: &PartitionedGraph,
) -> Result<CompletionEvidence, HistoryError> {
    let key = metadata_key(&[b"completion"]);
    let mut values = partitioned
        .metadata
        .iter()
        .filter(|(candidate, _)| candidate == &key);
    let (_, bytes) = values
        .next()
        .ok_or_else(|| HistoryError::InvalidArtifacts("missing completion evidence".to_owned()))?;
    if values.next().is_some() {
        return Err(HistoryError::InvalidArtifacts(
            "duplicate completion evidence".to_owned(),
        ));
    }
    let completion = decode_typed(bytes, "compass.metadata.completion")?;
    CompletionEvidence::validate(&completion)?;
    Ok(completion)
}

fn is_builtin_artifact(path: &str) -> bool {
    matches!(
        path,
        "graph.json" | ".graphify_analysis.json" | ".graphify_labels.json" | "manifest.json"
    )
}

fn validate_registry_declarations(registry: &[ArtifactRegistryEntry]) -> Result<(), HistoryError> {
    let mut paths = BTreeSet::new();
    for entry in registry {
        if entry.registry_version != 1 {
            return Err(HistoryError::InvalidArtifacts(format!(
                "unsupported artifact registry version {}",
                entry.registry_version
            )));
        }
        validate_relative_path(&entry.relative_path)?;
        if !paths.insert(entry.relative_path.as_str()) {
            return Err(HistoryError::InvalidArtifacts(format!(
                "duplicate artifact registry path {}",
                entry.relative_path
            )));
        }
        match entry.class {
            ArtifactClass::Authoritative => {
                let digest = entry.content_digest.ok_or_else(|| {
                    HistoryError::InvalidArtifacts(format!(
                        "authoritative artifact {} has no digest",
                        entry.relative_path
                    ))
                })?;
                if entry.regeneration_version.is_some() {
                    return Err(HistoryError::InvalidArtifacts(format!(
                        "authoritative artifact {} has a renderer",
                        entry.relative_path
                    )));
                }
                if let Some(bytes) = &entry.storage
                    && <[u8; 32]>::from(Sha256::digest(bytes)) != digest
                {
                    return Err(HistoryError::InvalidArtifacts(format!(
                        "stored artifact {} does not match its digest",
                        entry.relative_path
                    )));
                }
            }
            ArtifactClass::Derived => {
                if entry.regeneration_version.is_none()
                    || entry.content_digest.is_some()
                    || entry.storage.is_some()
                {
                    return Err(HistoryError::InvalidArtifacts(format!(
                        "derived artifact {} has an invalid registry declaration",
                        entry.relative_path
                    )));
                }
            }
            ArtifactClass::Operational => {
                if entry.content_digest.is_some()
                    || entry.storage.is_some()
                    || entry.regeneration_version.is_some()
                {
                    return Err(HistoryError::InvalidArtifacts(format!(
                        "operational artifact {} entered realization identity",
                        entry.relative_path
                    )));
                }
            }
        }
    }
    Ok(())
}

fn verify_registry_content(
    entry: &ArtifactRegistryEntry,
    bytes: &[u8],
) -> Result<(), HistoryError> {
    let digest: [u8; 32] = Sha256::digest(bytes).into();
    if entry.content_digest != Some(digest)
        || entry
            .storage
            .as_deref()
            .is_some_and(|stored| stored != bytes)
    {
        return Err(HistoryError::InvalidArtifacts(format!(
            "artifact {} does not match its registry entry",
            entry.relative_path
        )));
    }
    Ok(())
}

fn verify_builtin_registry_content(
    artifacts: &GraphArtifacts,
    registry: &[ArtifactRegistryEntry],
) -> Result<(), HistoryError> {
    for entry in registry
        .iter()
        .filter(|entry| entry.class == ArtifactClass::Authoritative)
    {
        let bytes = match entry.relative_path.as_str() {
            "graph.json" => Some(canonical_json_bytes(&serde_json::to_value(
                &artifacts.document,
            )?)?),
            ".graphify_analysis.json" => artifacts
                .analysis
                .as_ref()
                .map(canonical_json_bytes)
                .transpose()?,
            ".graphify_labels.json" => artifacts
                .labels
                .as_ref()
                .map(canonical_json_bytes)
                .transpose()?,
            "manifest.json" => artifacts
                .manifest
                .as_ref()
                .map(canonical_json_bytes)
                .transpose()?,
            _ => None,
        };
        if is_builtin_artifact(&entry.relative_path) {
            let bytes = bytes.ok_or_else(|| {
                HistoryError::InvalidArtifacts(format!(
                    "registry requires missing artifact {}",
                    entry.relative_path
                ))
            })?;
            verify_registry_content(entry, &bytes)?;
        }
    }
    Ok(())
}

fn hyperedge_array(value: Option<&Value>) -> Result<Vec<Value>, HistoryError> {
    match value {
        None => Ok(Vec::new()),
        Some(Value::Array(values)) => Ok(values.clone()),
        Some(_) => Err(HistoryError::InvalidArtifacts(
            "hyperedges must be an array".to_owned(),
        )),
    }
}

fn sort_unique(entries: &mut [(Vec<u8>, Vec<u8>)], kind: &str) -> Result<(), HistoryError> {
    entries.sort_by(|left, right| left.0.cmp(&right.0));
    if entries.windows(2).any(|pair| pair[0].0 == pair[1].0) {
        Err(HistoryError::InvalidArtifacts(format!(
            "duplicate {kind} record key"
        )))
    } else {
        Ok(())
    }
}

fn analysis_key(parts: &[&[u8]]) -> Vec<u8> {
    parts
        .iter()
        .fold(
            KeyBuilder::new()
                .push_segment(ANALYSIS_SCHEMA)
                .push_segment(ANALYSIS_KIND),
            |builder, part| builder.push_segment(part),
        )
        .finish()
}

fn metadata_key(parts: &[&[u8]]) -> Vec<u8> {
    parts
        .iter()
        .fold(
            KeyBuilder::new()
                .push_segment(METADATA_SCHEMA)
                .push_segment(METADATA_KIND),
            |builder, part| builder.push_segment(part),
        )
        .finish()
}

fn metadata_rank_key(kind: &str, rank: usize) -> Result<Vec<u8>, HistoryError> {
    let rank = u64::try_from(rank)
        .map_err(|_| HistoryError::InvalidArtifacts("record rank exceeds u64".to_owned()))?;
    Ok(metadata_key(&[kind.as_bytes(), &rank.to_be_bytes()]))
}

fn encode_record(schema: &str, value: &Value) -> Result<Vec<u8>, HistoryError> {
    let payload = canonical_json_bytes(value)?;
    VersionedValue::raw(schema, RECORD_VERSION, payload)
        .to_bytes()
        .map_err(HistoryError::from)
}

fn decode_record(bytes: &[u8], schema: &str) -> Result<Value, HistoryError> {
    let envelope = VersionedValue::from_bytes(bytes)?;
    envelope.require_schema(schema, RECORD_VERSION)?;
    serde_json::from_slice(&envelope.payload).map_err(HistoryError::from)
}

fn decode_typed<T: for<'de> Deserialize<'de>>(
    bytes: &[u8],
    schema: &str,
) -> Result<T, HistoryError> {
    serde_json::from_value(decode_record(bytes, schema)?).map_err(HistoryError::from)
}

fn decode_map<T: for<'de> Deserialize<'de>>(
    entries: &[(Vec<u8>, Vec<u8>)],
    schema: &str,
) -> Result<BTreeMap<Vec<u8>, T>, HistoryError> {
    entries
        .iter()
        .map(|(key, value)| Ok((key.clone(), decode_typed(value, schema)?)))
        .collect()
}

fn decode_value_map(
    entries: &[(Vec<u8>, Vec<u8>)],
    schema: &str,
) -> Result<BTreeMap<Vec<u8>, Value>, HistoryError> {
    entries
        .iter()
        .map(|(key, value)| Ok((key.clone(), decode_record(value, schema)?)))
        .collect()
}

fn rank_bytes(bytes: &[u8]) -> Result<u64, HistoryError> {
    let rank: [u8; 8] = bytes.try_into().map_err(|_| {
        HistoryError::InvalidArtifacts("order rank must contain eight bytes".to_owned())
    })?;
    Ok(u64::from_be_bytes(rank))
}

fn restore_order<T>(
    values: &mut BTreeMap<Vec<u8>, T>,
    order: BTreeMap<u64, OrderedRecord>,
    kind: &str,
) -> Result<Vec<T>, HistoryError> {
    let mut restored = Vec::with_capacity(order.len());
    for (expected, (actual, record)) in order.into_iter().enumerate() {
        if actual != u64::try_from(expected).unwrap_or(u64::MAX) {
            return Err(HistoryError::InvalidArtifacts(format!(
                "non-contiguous {kind} order"
            )));
        }
        restored.push(values.remove(&record.key).ok_or_else(|| {
            HistoryError::InvalidArtifacts(format!("{kind} order references a missing record"))
        })?);
    }
    if values.is_empty() {
        Ok(restored)
    } else {
        Err(HistoryError::InvalidArtifacts(format!(
            "{kind} records are missing order entries"
        )))
    }
}

fn restore_hyperedge_order(
    values: &mut BTreeMap<Vec<u8>, Value>,
    order: BTreeMap<u64, OrderedRecord>,
) -> Result<Vec<(HyperedgeLocation, Value)>, HistoryError> {
    let mut restored = Vec::with_capacity(order.len());
    for (expected, (actual, record)) in order.into_iter().enumerate() {
        if actual != u64::try_from(expected).unwrap_or(u64::MAX) {
            return Err(HistoryError::InvalidArtifacts(
                "non-contiguous hyperedge order".to_owned(),
            ));
        }
        let location = record.location.ok_or_else(|| {
            HistoryError::InvalidArtifacts("hyperedge order has no placement".to_owned())
        })?;
        let value = values.remove(&record.key).ok_or_else(|| {
            HistoryError::InvalidArtifacts("hyperedge order references a missing record".to_owned())
        })?;
        restored.push((location, value));
    }
    if values.is_empty() {
        Ok(restored)
    } else {
        Err(HistoryError::InvalidArtifacts(
            "hyperedge records are missing order entries".to_owned(),
        ))
    }
}

fn validate_sidecar_paths(
    sidecars: &BTreeMap<String, ArtifactContent>,
) -> Result<(), HistoryError> {
    for path in sidecars.keys() {
        validate_relative_path(path)?;
    }
    Ok(())
}

fn validate_relative_path(path: &str) -> Result<(), HistoryError> {
    let candidate = Path::new(path);
    if path.is_empty()
        || candidate.is_absolute()
        || candidate.components().any(|component| {
            matches!(
                component,
                Component::ParentDir
                    | Component::RootDir
                    | Component::Prefix(_)
                    | Component::CurDir
            )
        })
    {
        return Err(HistoryError::InvalidArtifacts(format!(
            "unsafe artifact path {}",
            candidate.display()
        )));
    }
    Ok(())
}

fn read_optional_json(path: &Path) -> Result<Option<Value>, HistoryError> {
    match fs::read(path) {
        Ok(bytes) => Ok(Some(serde_json::from_slice(&bytes)?)),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(source) => Err(crate::error::io_error(path, source)),
    }
}
