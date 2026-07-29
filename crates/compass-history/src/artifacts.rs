use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Component, Path};

use compass_analysis::{AnalysisBundle, FunctionSummary};
use compass_files::{write_bytes_atomic, write_json_atomic};
use compass_ir::{EvidenceRecord, FunctionIr, ModuleIr, ProgramBundle, ProviderDescriptor};
use compass_model::code_graph::GraphDocument as TrustedGraphDocument;
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
const TRUSTED_GRAPH_CONTENT: &str = ".compass-history/graph.v1.json";

/// All authoritative inputs needed to reconstruct a complete Compass output.
#[derive(Clone, Debug, PartialEq)]
pub struct GraphArtifacts {
    pub document: GraphDocument,
    pub program: Option<AnalysisBundle>,
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
    pub program_facts: Vec<(Vec<u8>, Vec<u8>)>,
    pub program_summaries: Vec<(Vec<u8>, Vec<u8>)>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
struct ProgramHeader {
    program_schema: String,
    analysis_schema_version: u32,
    analyzer_version: u32,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct DocumentHeader {
    directed: bool,
    multigraph: bool,
    graph: Map<String, Value>,
    extras: BTreeMap<String, Value>,
    graph_hyperedges_present: bool,
    top_hyperedges_present: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct OrderedRecord {
    key: Vec<u8>,
    #[serde(skip_serializing_if = "Option::is_none")]
    location: Option<HyperedgeLocation>,
}

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd, Serialize, Deserialize)]
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

    /// Consume and partition this completed output without retaining the source graph.
    pub fn into_partition(self) -> Result<PartitionedGraph, HistoryError> {
        self.artifacts.into_partition(&self.completion)
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

    /// Return the canonical durable graph artifact retained by this realization.
    pub fn graph_json_bytes(&self) -> Result<Vec<u8>, HistoryError> {
        authoritative_graph_bytes(self)
    }

    /// Return authoritative sidecars intended for product output.
    #[must_use]
    pub fn export_sidecars(&self) -> BTreeMap<String, ArtifactContent> {
        self.authoritative_sidecars
            .iter()
            .filter(|(path, _)| path.as_str() != TRUSTED_GRAPH_CONTENT)
            .map(|(path, content)| (path.clone(), content.clone()))
            .collect()
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
        let (document, trusted_graph) = load_trusted_graph(&output_dir.join("graph.json"))?;
        authoritative_sidecars.insert(TRUSTED_GRAPH_CONTENT.to_owned(), trusted_graph);
        let artifacts = Self {
            document,
            program: read_optional_program(&output_dir.join("program.json"))?,
            analysis: read_optional_json(&output_dir.join(".compass_analysis.json"))?,
            labels: read_optional_json(&output_dir.join(".compass_labels.json"))?,
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
        self.clone().into_partition(completion)
    }

    /// Consume realization state while producing deterministic typed records.
    pub fn into_partition(
        mut self,
        completion: &CompletionEvidence,
    ) -> Result<PartitionedGraph, HistoryError> {
        completion.validate()?;
        validate_sidecar_paths(&self.authoritative_sidecars)?;
        let trusted_graph = self
            .authoritative_sidecars
            .get(TRUSTED_GRAPH_CONTENT)
            .map(|bytes| serde_json::from_slice::<TrustedGraphDocument>(bytes))
            .transpose()?;
        canonicalize_graph_document(&mut self.document)?;
        let registry = artifact_registry_from_canonical(&self)?;
        let mut partitioned = PartitionedGraph::default();

        if let Some(program) = self.program.take() {
            program.validate()?;
            let AnalysisBundle {
                analysis_schema_version,
                analyzer_version,
                program,
                summaries,
                reverse_calls,
            } = program;
            let compass_ir::ProgramBundle {
                schema,
                providers,
                evidence,
                modules,
            } = program;
            partitioned.program_facts.push((
                program_key("header", "analysis"),
                encode_record(
                    "compass.program.header",
                    &serde_json::to_value(ProgramHeader {
                        program_schema: schema,
                        analysis_schema_version,
                        analyzer_version,
                    })?,
                )?,
            ));
            for provider in providers {
                let key = program_key("provider", &provider.id);
                partitioned.program_facts.push((
                    key,
                    encode_record("compass.program.provider", &serde_json::to_value(provider)?)?,
                ));
            }
            for evidence in evidence {
                let key = program_key("evidence", &evidence.id);
                partitioned.program_facts.push((
                    key,
                    encode_record("compass.program.evidence", &serde_json::to_value(evidence)?)?,
                ));
            }
            for module in modules {
                let key = program_key("module", &module.source_file);
                partitioned.program_facts.push((
                    key,
                    encode_record("compass.program.module", &serde_json::to_value(&module)?)?,
                ));
                for function in module.functions {
                    let key = program_key("function", &function.symbol_id);
                    partitioned.program_facts.push((
                        key,
                        encode_record(
                            "compass.program.function",
                            &serde_json::to_value(function)?,
                        )?,
                    ));
                }
            }
            for summary in summaries {
                let key = program_key("summary", &summary.symbol_id);
                partitioned.program_summaries.push((
                    key,
                    encode_record("compass.program.summary", &serde_json::to_value(summary)?)?,
                ));
            }
            for (target, callers) in reverse_calls {
                partitioned.program_summaries.push((
                    program_key("reverse-call", &target),
                    encode_record(
                        "compass.program.reverse-call",
                        &serde_json::to_value(callers)?,
                    )?,
                ));
            }
        }

        // The owned document was canonicalized before its registry digest was
        // computed, so records can consume that exact order without retaining
        // a second graph-sized allocation.
        if let Some(trusted) = &trusted_graph {
            for (rank, node) in trusted.nodes.iter().enumerate() {
                let key = node_key(&node.id);
                partitioned.nodes.push((
                    key.clone(),
                    encode_record("compass.graph.node.v1", &serde_json::to_value(node)?)?,
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
        } else {
            for (rank, mut node) in std::mem::take(&mut self.document.nodes)
                .into_iter()
                .enumerate()
            {
                for field in MOVED_NODE_FIELDS {
                    if let Some(value) = node.attributes.remove(field) {
                        partitioned.analysis.push((
                            analysis_key(&[b"node", node.id.as_bytes(), field.as_bytes()]),
                            encode_record("compass.analysis.node", &value)?,
                        ));
                    }
                }
                let key = node_key(&node.id);
                partitioned.nodes.push((
                    key.clone(),
                    encode_record("compass.node", &serde_json::to_value(node)?)?,
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
        }

        let mut edge_occurrences = BTreeMap::<Vec<u8>, u64>::new();
        if let Some(trusted) = &trusted_graph {
            for (rank, edge) in trusted.links.iter().enumerate() {
                let key = edge_key(
                    &edge.source,
                    &edge.target,
                    edge.kind.as_str(),
                    true,
                    Some(edge.id.as_bytes()),
                );
                partitioned.edges.push((
                    key.clone(),
                    encode_record("compass.graph.edge.v1", &serde_json::to_value(edge)?)?,
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
        } else {
            for (rank, edge) in std::mem::take(&mut self.document.links)
                .into_iter()
                .enumerate()
            {
                let canonical = canonical_json_bytes(&serde_json::to_value(&edge)?)?;
                let discriminator = edge_discriminator(
                    &edge,
                    self.document.multigraph,
                    &canonical,
                    &mut edge_occurrences,
                )?;
                let (source, target) = edge_identity_endpoints(&edge);
                let key = edge_key(
                    source,
                    target,
                    &edge.string("relation"),
                    true,
                    discriminator.as_deref(),
                );
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
        }

        let graph_hyperedges_present = self.document.graph.contains_key("hyperedges");
        let graph_hyperedges = take_hyperedge_array(self.document.graph.remove("hyperedges"))?;
        let top_hyperedges_present = self.document.extras.contains_key("hyperedges");
        let top_hyperedges = take_hyperedge_array(self.document.extras.remove("hyperedges"))?;
        let mut hyperedge_occurrences = BTreeMap::<Vec<u8>, u64>::new();
        let mut explicit_hyperedges = BTreeSet::<Vec<u8>>::new();
        let mut ordered_hyperedges = graph_hyperedges
            .into_iter()
            .map(|value| (HyperedgeLocation::Graph, value))
            .chain(
                top_hyperedges
                    .into_iter()
                    .map(|value| (HyperedgeLocation::TopLevel, value)),
            )
            .map(|(location, value)| Ok((location, canonical_json_bytes(&value)?, value)))
            .collect::<Result<Vec<_>, HistoryError>>()?;
        ordered_hyperedges
            .sort_by(|left, right| left.0.cmp(&right.0).then_with(|| left.1.cmp(&right.1)));
        for (rank, (location, canonical, hyperedge)) in ordered_hyperedges.into_iter().enumerate() {
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
            partitioned
                .hyperedges
                .push((key.clone(), encode_record("compass.hyperedge", &hyperedge)?));
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

        partitioned.metadata.push((
            metadata_key(&[b"document"]),
            encode_record(
                "compass.metadata.document",
                &serde_json::to_value(DocumentHeader {
                    directed: self.document.directed,
                    multigraph: self.document.multigraph,
                    graph: std::mem::take(&mut self.document.graph),
                    extras: std::mem::take(&mut self.document.extras),
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
            ".compass_analysis.json",
            self.analysis.take(),
        )?;
        add_optional_analysis(&mut partitioned, ".compass_labels.json", self.labels.take())?;
        if let Some(manifest) = self.manifest.take() {
            let manifest = canonical_manifest_owned(manifest);
            partitioned.metadata.push((
                metadata_key(&[b"manifest"]),
                encode_record("compass.metadata.manifest", &manifest)?,
            ));
        }
        for (path, bytes) in std::mem::take(&mut self.authoritative_sidecars) {
            let key = metadata_key(&[b"sidecar", path.as_bytes()]);
            partitioned.metadata.push((
                key,
                encode_record("compass.metadata.sidecar", &serde_json::to_value(bytes)?)?,
            ));
        }
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
        sort_unique(&mut partitioned.program_facts, "program fact")?;
        sort_unique(&mut partitioned.program_summaries, "program summary")?;
        Ok(partitioned)
    }

    /// Reconstruct the exact supported graph structure and authoritative sidecars.
    pub fn reconstruct(partitioned: &PartitionedGraph) -> Result<Self, HistoryError> {
        let program =
            reconstruct_program(&partitioned.program_facts, &partitioned.program_summaries)?;
        let mut nodes = decode_node_map(&partitioned.nodes)?;
        let mut edges = decode_edge_map(&partitioned.edges)?;
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
                        b".compass_analysis.json" => analysis = Some(value),
                        b".compass_labels.json" => labels = Some(value),
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
            },
            program,
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
        if let Some(trusted) = self.authoritative_sidecars.get(TRUSTED_GRAPH_CONTENT) {
            write_bytes_atomic(output_dir.join("graph.json"), trusted)?;
        } else {
            write_json_atomic(output_dir.join("graph.json"), &self.document, false)?;
        }
        if let Some(program) = &self.program {
            write_bytes_atomic(output_dir.join("program.json"), &program.canonical_bytes()?)?;
        }
        if let Some(value) = &self.analysis {
            write_json_atomic(output_dir.join(".compass_analysis.json"), value, false)?;
        }
        if let Some(value) = &self.labels {
            write_json_atomic(output_dir.join(".compass_labels.json"), value, false)?;
        }
        if let Some(value) = &self.manifest {
            write_json_atomic(output_dir.join("manifest.json"), value, false)?;
        }
        for (path, bytes) in &self.authoritative_sidecars {
            if path == TRUSTED_GRAPH_CONTENT {
                continue;
            }
            let destination = output_dir.join(path);
            if let Some(parent) = destination.parent() {
                fs::create_dir_all(parent)
                    .map_err(|source| crate::error::io_error(parent, source))?;
            }
            write_bytes_atomic(destination, bytes)?;
        }
        write_json_atomic(
            output_dir.join(".compass_semantic_marker"),
            &SemanticCompletionMarker::from(completion),
            false,
        )?;
        Ok(())
    }
}

fn load_trusted_graph(path: &Path) -> Result<(GraphDocument, Vec<u8>), HistoryError> {
    let trusted = TrustedGraphDocument::load_for_recluster(path)?;
    let trusted_bytes = canonical_json_bytes(&serde_json::to_value(&trusted)?)?;
    let graph = serde_json::to_value(&trusted.graph)?
        .as_object()
        .cloned()
        .unwrap_or_default();
    let nodes = trusted.nodes.iter().map(compat_node).collect();
    let links = trusted.links.iter().map(compat_edge).collect();
    Ok((
        GraphDocument {
            directed: trusted.directed,
            multigraph: trusted.multigraph,
            graph,
            nodes,
            links,
            extras: BTreeMap::new(),
        },
        trusted_bytes,
    ))
}

fn compat_node(node: &compass_model::code_graph::NodeRecord) -> NodeRecord {
    NodeRecord {
        id: node.id.clone(),
        attributes: node
            .properties()
            .filter(|(key, _)| *key != "id")
            .map(|(key, value)| (key.to_owned(), value))
            .collect(),
    }
}

fn compat_edge(edge: &compass_model::code_graph::EdgeRecord) -> EdgeRecord {
    EdgeRecord {
        source: edge.source.clone(),
        target: edge.target.clone(),
        attributes: edge
            .properties()
            .filter(|(key, _)| !matches!(*key, "source" | "target"))
            .map(|(key, value)| (key.to_owned(), value))
            .collect(),
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

fn edge_identity_endpoints(edge: &EdgeRecord) -> (&str, &str) {
    if let (Some(source), Some(target)) = (
        edge.attributes.get("_src").and_then(Value::as_str),
        edge.attributes.get("_tgt").and_then(Value::as_str),
    ) && ((source == edge.source && target == edge.target)
        || (source == edge.target && target == edge.source))
    {
        return (source, target);
    }
    (&edge.source, &edge.target)
}

fn add_optional_analysis(
    partitioned: &mut PartitionedGraph,
    path: &str,
    value: Option<Value>,
) -> Result<(), HistoryError> {
    if let Some(value) = value {
        partitioned.analysis.push((
            analysis_key(&[b"sidecar", path.as_bytes()]),
            encode_record("compass.analysis.sidecar", &value)?,
        ));
    }
    Ok(())
}

fn artifact_registry(
    artifacts: &GraphArtifacts,
) -> Result<Vec<ArtifactRegistryEntry>, HistoryError> {
    let graph_bytes = authoritative_graph_bytes(artifacts)?;
    artifact_registry_with_graph_bytes(artifacts, &graph_bytes)
}

fn artifact_registry_from_canonical(
    artifacts: &GraphArtifacts,
) -> Result<Vec<ArtifactRegistryEntry>, HistoryError> {
    let graph_bytes = artifacts
        .authoritative_sidecars
        .get(TRUSTED_GRAPH_CONTENT)
        .cloned()
        .map(Ok)
        .unwrap_or_else(|| canonical_json_bytes(&serde_json::to_value(&artifacts.document)?))?;
    artifact_registry_with_graph_bytes(artifacts, &graph_bytes)
}

fn authoritative_graph_bytes(artifacts: &GraphArtifacts) -> Result<Vec<u8>, HistoryError> {
    artifacts
        .authoritative_sidecars
        .get(TRUSTED_GRAPH_CONTENT)
        .cloned()
        .map(Ok)
        .unwrap_or_else(|| canonical_graph_bytes(&artifacts.document))
}

fn artifact_registry_with_graph_bytes(
    artifacts: &GraphArtifacts,
    graph_bytes: &[u8],
) -> Result<Vec<ArtifactRegistryEntry>, HistoryError> {
    let mut registry = vec![authoritative_entry(
        "graph.json",
        "application/json",
        graph_bytes,
    )];
    if let Some(program) = &artifacts.program {
        registry.push(authoritative_entry(
            "program.json",
            "application/json",
            &program.canonical_bytes()?,
        ));
    }
    for (path, value) in [
        (".compass_analysis.json", artifacts.analysis.as_ref()),
        (".compass_labels.json", artifacts.labels.as_ref()),
        ("manifest.json", artifacts.manifest.as_ref()),
    ] {
        if let Some(value) = value {
            let canonical;
            let value = if path == "manifest.json" {
                canonical = canonical_manifest(value);
                &canonical
            } else {
                value
            };
            registry.push(authoritative_entry(
                path,
                "application/json",
                &canonical_json_bytes(value)?,
            ));
        }
    }
    for (path, bytes) in &artifacts.authoritative_sidecars {
        if path == TRUSTED_GRAPH_CONTENT {
            continue;
        }
        let mut entry = authoritative_entry(path, "application/octet-stream", bytes);
        entry.storage = Some(bytes.clone());
        registry.push(entry);
    }
    for path in [
        "GRAPH_REPORT.md",
        "graph.html",
        "GRAPH_TREE.html",
        ".compass_labels.json.sig",
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
        relative_path: ".compass_semantic_marker".to_owned(),
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
        "graph.json"
            | "program.json"
            | ".compass_analysis.json"
            | ".compass_labels.json"
            | "manifest.json"
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
            "graph.json" => Some(authoritative_graph_bytes(artifacts)?),
            ".compass_analysis.json" => artifacts
                .analysis
                .as_ref()
                .map(canonical_json_bytes)
                .transpose()?,
            ".compass_labels.json" => artifacts
                .labels
                .as_ref()
                .map(canonical_json_bytes)
                .transpose()?,
            "manifest.json" => artifacts
                .manifest
                .as_ref()
                .map(|manifest| canonical_json_bytes(&canonical_manifest(manifest)))
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

fn canonical_graph_bytes(document: &GraphDocument) -> Result<Vec<u8>, HistoryError> {
    let mut canonical = document.clone();
    canonicalize_graph_document(&mut canonical)?;
    canonical_json_bytes(&serde_json::to_value(canonical)?)
}

fn canonicalize_graph_document(document: &mut GraphDocument) -> Result<(), HistoryError> {
    document
        .nodes
        .sort_by(|left, right| left.id.as_bytes().cmp(right.id.as_bytes()));
    let mut links = std::mem::take(&mut document.links)
        .into_iter()
        .map(|edge| Ok((canonical_json_bytes(&serde_json::to_value(&edge)?)?, edge)))
        .collect::<Result<Vec<_>, HistoryError>>()?;
    links.sort_by(|left, right| left.0.cmp(&right.0));
    document.links = links.into_iter().map(|(_, edge)| edge).collect();
    canonicalize_hyperedge_array(document.graph.get_mut("hyperedges"))?;
    canonicalize_hyperedge_array(document.extras.get_mut("hyperedges"))?;
    Ok(())
}

fn canonical_manifest(manifest: &Value) -> Value {
    canonical_manifest_owned(manifest.clone())
}

fn canonical_manifest_owned(mut canonical: Value) -> Value {
    let Some(entries) = canonical.as_object_mut() else {
        return canonical;
    };
    for entry in entries.values_mut() {
        if let Some(fields) = entry.as_object_mut() {
            fields.insert("mtime".to_owned(), Value::from(0));
        } else if entry.is_number() {
            *entry = Value::from(0);
        }
    }
    canonical
}

fn canonicalize_hyperedge_array(value: Option<&mut Value>) -> Result<(), HistoryError> {
    let Some(value) = value else {
        return Ok(());
    };
    let values = value
        .as_array_mut()
        .ok_or_else(|| HistoryError::InvalidArtifacts("hyperedges must be an array".to_owned()))?;
    let mut canonical = values
        .drain(..)
        .map(|value| Ok((canonical_json_bytes(&value)?, value)))
        .collect::<Result<Vec<_>, HistoryError>>()?;
    canonical.sort_by(|left, right| left.0.cmp(&right.0));
    values.extend(canonical.into_iter().map(|(_, value)| value));
    Ok(())
}

fn take_hyperedge_array(value: Option<Value>) -> Result<Vec<Value>, HistoryError> {
    match value {
        None => Ok(Vec::new()),
        Some(Value::Array(values)) => Ok(values),
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

pub(crate) fn program_key(kind: &str, identity: &str) -> Vec<u8> {
    KeyBuilder::new().push_str(kind).push_str(identity).finish()
}

fn reconstruct_program(
    facts: &[(Vec<u8>, Vec<u8>)],
    summaries: &[(Vec<u8>, Vec<u8>)],
) -> Result<Option<AnalysisBundle>, HistoryError> {
    if facts.is_empty() && summaries.is_empty() {
        return Ok(None);
    }
    let mut header = None;
    let mut providers = Vec::<ProviderDescriptor>::new();
    let mut evidence = Vec::<EvidenceRecord>::new();
    let mut modules = Vec::<ModuleIr>::new();
    let mut indexed_functions = BTreeMap::<String, FunctionIr>::new();
    for (key, bytes) in facts {
        let segments = decode_segments(key)
            .map_err(|error| HistoryError::InvalidArtifacts(error.to_string()))?;
        let [kind, identity] = segments.as_slice() else {
            return Err(HistoryError::InvalidArtifacts(
                "invalid program fact key".to_owned(),
            ));
        };
        let identity = std::str::from_utf8(identity)
            .map_err(|error| HistoryError::InvalidArtifacts(error.to_string()))?;
        match kind.as_slice() {
            b"header" if identity == "analysis" => {
                if header
                    .replace(decode_typed(bytes, "compass.program.header")?)
                    .is_some()
                {
                    return Err(HistoryError::InvalidArtifacts(
                        "duplicate program header".to_owned(),
                    ));
                }
            }
            b"provider" => {
                let value: ProviderDescriptor = decode_typed(bytes, "compass.program.provider")?;
                if value.id != identity {
                    return Err(HistoryError::InvalidArtifacts(
                        "program provider key does not match its ID".to_owned(),
                    ));
                }
                providers.push(value);
            }
            b"evidence" => {
                let value: EvidenceRecord = decode_typed(bytes, "compass.program.evidence")?;
                if value.id != identity {
                    return Err(HistoryError::InvalidArtifacts(
                        "program evidence key does not match its ID".to_owned(),
                    ));
                }
                evidence.push(value);
            }
            b"module" => {
                let value: ModuleIr = decode_typed(bytes, "compass.program.module")?;
                if value.source_file != identity {
                    return Err(HistoryError::InvalidArtifacts(
                        "program module key does not match its source".to_owned(),
                    ));
                }
                modules.push(value);
            }
            b"function" => {
                let value: FunctionIr = decode_typed(bytes, "compass.program.function")?;
                if value.symbol_id != identity {
                    return Err(HistoryError::InvalidArtifacts(
                        "program function key does not match its symbol".to_owned(),
                    ));
                }
                if indexed_functions
                    .insert(identity.to_owned(), value)
                    .is_some()
                {
                    return Err(HistoryError::InvalidArtifacts(
                        "duplicate indexed program function".to_owned(),
                    ));
                }
            }
            _ => {
                return Err(HistoryError::InvalidArtifacts(
                    "unknown program fact key".to_owned(),
                ));
            }
        }
    }
    let header: ProgramHeader = header
        .ok_or_else(|| HistoryError::InvalidArtifacts("missing program header".to_owned()))?;
    let module_functions = modules
        .iter()
        .flat_map(|module| &module.functions)
        .map(|function| (function.symbol_id.clone(), function.clone()))
        .collect::<BTreeMap<_, _>>();
    if indexed_functions != module_functions {
        return Err(HistoryError::InvalidArtifacts(
            "indexed program functions do not match module contents".to_owned(),
        ));
    }
    let mut function_summaries = Vec::<FunctionSummary>::new();
    let mut reverse_calls = BTreeMap::<String, Vec<String>>::new();
    for (key, bytes) in summaries {
        let segments = decode_segments(key)
            .map_err(|error| HistoryError::InvalidArtifacts(error.to_string()))?;
        let [kind, identity] = segments.as_slice() else {
            return Err(HistoryError::InvalidArtifacts(
                "invalid program summary key".to_owned(),
            ));
        };
        let identity = std::str::from_utf8(identity)
            .map_err(|error| HistoryError::InvalidArtifacts(error.to_string()))?;
        match kind.as_slice() {
            b"summary" => {
                let value: FunctionSummary = decode_typed(bytes, "compass.program.summary")?;
                if value.symbol_id != identity {
                    return Err(HistoryError::InvalidArtifacts(
                        "program summary key does not match its symbol".to_owned(),
                    ));
                }
                function_summaries.push(value);
            }
            b"reverse-call" => {
                let callers = decode_typed(bytes, "compass.program.reverse-call")?;
                if reverse_calls.insert(identity.to_owned(), callers).is_some() {
                    return Err(HistoryError::InvalidArtifacts(
                        "duplicate reverse-call target".to_owned(),
                    ));
                }
            }
            _ => {
                return Err(HistoryError::InvalidArtifacts(
                    "unknown program summary key".to_owned(),
                ));
            }
        }
    }
    let bundle = AnalysisBundle {
        analysis_schema_version: header.analysis_schema_version,
        analyzer_version: header.analyzer_version,
        program: ProgramBundle {
            schema: header.program_schema,
            providers,
            evidence,
            modules,
        },
        summaries: function_summaries,
        reverse_calls,
    }
    .canonicalized();
    bundle.validate()?;
    Ok(Some(bundle))
}

fn metadata_rank_key(kind: &str, rank: usize) -> Result<Vec<u8>, HistoryError> {
    let rank = u64::try_from(rank)
        .map_err(|_| HistoryError::InvalidArtifacts("record rank exceeds u64".to_owned()))?;
    Ok(metadata_key(&[kind.as_bytes(), &rank.to_be_bytes()]))
}

fn encode_record(schema: &str, value: &Value) -> Result<Vec<u8>, HistoryError> {
    crate::validate::validate_generated_json(value)?;
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

pub(crate) fn decode_typed<T: for<'de> Deserialize<'de>>(
    bytes: &[u8],
    schema: &str,
) -> Result<T, HistoryError> {
    serde_json::from_value(decode_record(bytes, schema)?).map_err(HistoryError::from)
}

fn decode_node_map(
    entries: &[(Vec<u8>, Vec<u8>)],
) -> Result<BTreeMap<Vec<u8>, NodeRecord>, HistoryError> {
    entries
        .iter()
        .map(|(key, bytes)| {
            let envelope = VersionedValue::from_bytes(bytes)?;
            let node = match envelope.schema.as_str() {
                "compass.node" => serde_json::from_slice(&envelope.payload)?,
                "compass.graph.node.v1" => {
                    let typed = serde_json::from_slice::<compass_model::code_graph::NodeRecord>(
                        &envelope.payload,
                    )?;
                    compat_node(&typed)
                }
                schema => {
                    return Err(HistoryError::InvalidArtifacts(format!(
                        "unexpected node record schema {schema}"
                    )));
                }
            };
            Ok((key.clone(), node))
        })
        .collect()
}

fn decode_edge_map(
    entries: &[(Vec<u8>, Vec<u8>)],
) -> Result<BTreeMap<Vec<u8>, EdgeRecord>, HistoryError> {
    entries
        .iter()
        .map(|(key, bytes)| {
            let envelope = VersionedValue::from_bytes(bytes)?;
            let edge = match envelope.schema.as_str() {
                "compass.edge" => serde_json::from_slice(&envelope.payload)?,
                "compass.graph.edge.v1" => {
                    let typed = serde_json::from_slice::<compass_model::code_graph::EdgeRecord>(
                        &envelope.payload,
                    )?;
                    compat_edge(&typed)
                }
                schema => {
                    return Err(HistoryError::InvalidArtifacts(format!(
                        "unexpected edge record schema {schema}"
                    )));
                }
            };
            Ok((key.clone(), edge))
        })
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
        || path.contains('\\')
        || path
            .split('/')
            .any(|component| component.is_empty() || matches!(component, "." | ".."))
        || (path.as_bytes().get(1) == Some(&b':') && path.as_bytes()[0].is_ascii_alphabetic())
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

fn read_optional_program(path: &Path) -> Result<Option<AnalysisBundle>, HistoryError> {
    let bytes = match fs::read(path) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(source) => return Err(crate::error::io_error(path, source)),
    };
    let program: AnalysisBundle = serde_json::from_slice(&bytes)?;
    let canonical = program.canonical_bytes()?;
    if canonical != bytes {
        return Err(HistoryError::InvalidArtifacts(
            "program.json is not canonical".to_owned(),
        ));
    }
    Ok(Some(program))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn registry_entry(class: ArtifactClass, path: &str) -> ArtifactRegistryEntry {
        ArtifactRegistryEntry {
            registry_version: 1,
            relative_path: path.to_owned(),
            class,
            media_type: "application/json".to_owned(),
            schema_version: Some(1),
            content_digest: None,
            storage: None,
            regeneration_version: None,
        }
    }

    #[test]
    fn registry_declarations_reject_every_invalid_storage_combination() {
        let bytes = br#"{"ok":true}"#.to_vec();
        let digest: [u8; 32] = Sha256::digest(&bytes).into();
        let mut authoritative = registry_entry(ArtifactClass::Authoritative, "facts.json");
        authoritative.content_digest = Some(digest);
        authoritative.storage = Some(bytes.clone());
        assert!(validate_registry_declarations(&[authoritative.clone()]).is_ok());
        assert!(verify_registry_content(&authoritative, &bytes).is_ok());
        assert!(verify_registry_content(&authoritative, b"different").is_err());

        let mut invalid = authoritative.clone();
        invalid.registry_version = 2;
        assert!(validate_registry_declarations(&[invalid]).is_err());
        let mut invalid = authoritative.clone();
        invalid.content_digest = None;
        assert!(validate_registry_declarations(&[invalid]).is_err());
        let mut invalid = authoritative.clone();
        invalid.regeneration_version = Some("renderer-v1".to_owned());
        assert!(validate_registry_declarations(&[invalid]).is_err());
        let mut invalid = authoritative.clone();
        invalid.storage = Some(b"different".to_vec());
        assert!(validate_registry_declarations(&[invalid]).is_err());
        let duplicate = authoritative.clone();
        assert!(validate_registry_declarations(&[authoritative, duplicate]).is_err());

        let mut derived = registry_entry(ArtifactClass::Derived, "report.html");
        assert!(validate_registry_declarations(&[derived.clone()]).is_err());
        derived.regeneration_version = Some("html-v1".to_owned());
        assert!(validate_registry_declarations(&[derived.clone()]).is_ok());
        derived.content_digest = Some(digest);
        assert!(validate_registry_declarations(&[derived]).is_err());

        let operational = registry_entry(ArtifactClass::Operational, "attempt.log");
        assert!(validate_registry_declarations(std::slice::from_ref(&operational)).is_ok());
        let mut invalid = operational;
        invalid.storage = Some(bytes);
        assert!(validate_registry_declarations(&[invalid]).is_err());
    }

    #[test]
    fn artifact_paths_arrays_and_ordering_fail_closed() -> Result<(), Box<dyn std::error::Error>> {
        for path in [
            "",
            "/absolute",
            "../escape",
            "a/./b",
            "a//b",
            "a\\b",
            "C:/escape",
            "C:escape",
        ] {
            assert!(validate_relative_path(path).is_err(), "accepted {path:?}");
        }
        assert!(validate_relative_path("nested/facts.json").is_ok());
        assert!(is_builtin_artifact("graph.json"));
        assert!(is_builtin_artifact(".compass_analysis.json"));
        assert!(is_builtin_artifact(".compass_labels.json"));
        assert!(is_builtin_artifact("manifest.json"));
        assert!(!is_builtin_artifact("custom.json"));

        assert!(take_hyperedge_array(None)?.is_empty());
        assert_eq!(take_hyperedge_array(Some(json!([{"id":"h"}])))?.len(), 1);
        assert!(take_hyperedge_array(Some(json!({"id":"h"}))).is_err());

        let mut unique = vec![(b"b".to_vec(), vec![]), (b"a".to_vec(), vec![])];
        sort_unique(&mut unique, "node")?;
        assert_eq!(unique[0].0, b"a");
        let mut duplicate = vec![(b"a".to_vec(), vec![]), (b"a".to_vec(), vec![])];
        assert!(sort_unique(&mut duplicate, "node").is_err());
        assert_eq!(rank_bytes(&7_u64.to_be_bytes())?, 7);
        assert!(rank_bytes(&[0; 7]).is_err());
        assert_eq!(decode_segments(&metadata_rank_key("node", 2)?)?.len(), 4);
        Ok(())
    }

    #[test]
    fn ordered_record_reconstruction_rejects_gaps_missing_records_and_leftovers()
    -> Result<(), Box<dyn std::error::Error>> {
        let key_a = b"a".to_vec();
        let key_b = b"b".to_vec();
        let ordered = |key: Vec<u8>| OrderedRecord {
            key,
            location: None,
        };
        let mut values = BTreeMap::from([(key_a.clone(), 1), (key_b.clone(), 2)]);
        let order = BTreeMap::from([(0, ordered(key_a.clone())), (1, ordered(key_b.clone()))]);
        assert_eq!(restore_order(&mut values, order, "node")?, [1, 2]);

        let mut values = BTreeMap::from([(key_a.clone(), 1)]);
        assert!(
            restore_order(
                &mut values,
                BTreeMap::from([(1, ordered(key_a.clone()))]),
                "node"
            )
            .is_err()
        );
        let mut values = BTreeMap::from([(key_a.clone(), 1)]);
        assert!(
            restore_order(
                &mut values,
                BTreeMap::from([(0, ordered(key_b.clone()))]),
                "node"
            )
            .is_err()
        );
        let mut values = BTreeMap::from([(key_a.clone(), 1), (key_b.clone(), 2)]);
        assert!(
            restore_order(
                &mut values,
                BTreeMap::from([(0, ordered(key_a.clone()))]),
                "node"
            )
            .is_err()
        );

        let placed = |key: Vec<u8>, location| OrderedRecord { key, location };
        let mut values = BTreeMap::from([(key_a.clone(), json!({"id":"h"}))]);
        let restored = restore_hyperedge_order(
            &mut values,
            BTreeMap::from([(0, placed(key_a.clone(), Some(HyperedgeLocation::Graph)))]),
        )?;
        assert_eq!(restored.len(), 1);

        let mut values = BTreeMap::from([(key_a.clone(), json!(1))]);
        assert!(
            restore_hyperedge_order(
                &mut values,
                BTreeMap::from([(0, placed(key_a.clone(), None))])
            )
            .is_err()
        );
        let mut values = BTreeMap::from([(key_a.clone(), json!(1))]);
        assert!(
            restore_hyperedge_order(
                &mut values,
                BTreeMap::from([(0, placed(key_b.clone(), Some(HyperedgeLocation::TopLevel)))])
            )
            .is_err()
        );
        let mut values = BTreeMap::from([(key_a.clone(), json!(1))]);
        assert!(restore_hyperedge_order(&mut values, BTreeMap::new()).is_err());
        let mut values = BTreeMap::from([(key_a.clone(), json!(1))]);
        assert!(
            restore_hyperedge_order(
                &mut values,
                BTreeMap::from([(1, placed(key_a, Some(HyperedgeLocation::TopLevel)))])
            )
            .is_err()
        );
        Ok(())
    }

    #[test]
    fn completion_and_optional_json_boundaries_are_explicit()
    -> Result<(), Box<dyn std::error::Error>> {
        let completion = CompletionEvidence {
            extraction_succeeded: true,
            allow_partial: false,
            semantic_files_expected: 0,
            semantic_files_completed: 0,
            failed_chunks: 0,
        };
        let encoded = encode_record(
            "compass.metadata.completion",
            &serde_json::to_value(&completion)?,
        )?;
        let key = metadata_key(&[b"completion"]);
        let missing = PartitionedGraph::default();
        assert!(completion_from_partition(&missing).is_err());
        let duplicate = PartitionedGraph {
            metadata: vec![(key.clone(), encoded.clone()), (key, encoded)],
            ..PartitionedGraph::default()
        };
        assert!(completion_from_partition(&duplicate).is_err());

        let directory = tempfile::tempdir()?;
        let missing = directory.path().join("missing.json");
        assert_eq!(read_optional_json(&missing)?, None);
        let valid = directory.path().join("valid.json");
        fs::write(&valid, b"{\"ok\":true}")?;
        assert_eq!(read_optional_json(&valid)?, Some(json!({"ok": true})));
        let invalid = directory.path().join("invalid.json");
        fs::write(&invalid, b"{")?;
        assert!(read_optional_json(&invalid).is_err());
        assert!(read_optional_json(directory.path()).is_err());
        Ok(())
    }
}
