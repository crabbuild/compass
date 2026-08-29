use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Component, Path};
use std::time::Instant;

use compass_analysis::{AnalysisBundle, FunctionSummary};
use compass_files::{write_bytes_atomic, write_json_atomic};
use compass_ir::{EvidenceRecord, FunctionIr, ModuleIr, ProgramBundle, ProviderDescriptor};
use compass_model::code_graph::GraphDocument as TrustedGraphDocument;
use compass_model::{EdgeRecord, GraphDocument, NodeRecord};
pub use compass_partition::PartitionedGraph;
use prolly::{KeyBuilder, VersionedValue, decode_segments};
use rayon::prelude::*;
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
const NODE_COMPATIBILITY_FIELDS: [&str; 14] = [
    "label",
    "qualified_name",
    "file_type",
    "source_file",
    "source_location",
    "line_start",
    "line_end",
    "signature",
    "signature_hash",
    "implementation_hash",
    "source_hash",
    "_origin",
    "confidence",
    "community_name",
];
const EDGE_COMPATIBILITY_FIELDS: [&str; 6] = [
    "relation",
    "source_file",
    "source_location",
    "confidence",
    "confidence_score",
    "_origin",
];
const TRUSTED_GRAPH_CONTENT: &str = "history/graph.v1.json";
const PROGRAM_SOURCE_DIGEST_CONTENT: &str = "history/program.source-digest";
const SOURCE_INVENTORY_CONTENT: &str = "source-inventory.json";

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

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
struct ProgramHeader {
    program_schema: String,
    analysis_schema_version: u32,
    analyzer_version: u32,
}

struct LoadedProgram {
    bundle: Option<AnalysisBundle>,
    source_digest: Option<[u8; 32]>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub(crate) struct IndexedProgramModule {
    pub(crate) module: ModuleIr,
    pub(crate) function_ids: Vec<String>,
}

pub(crate) enum DecodedProgramModule {
    Embedded(ModuleIr),
    Indexed(IndexedProgramModule),
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

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
struct TrustedGraphMarker {
    schema: String,
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

    /// Load completed output for immediate publication validation.
    ///
    /// Program canonical and semantic validation is deferred to partitioning,
    /// where the source digest is checked against the one canonical encoding.
    /// Callers must not inspect or publish the returned artifacts without first
    /// passing through the normal history publication path.
    #[doc(hidden)]
    pub fn load_for_publication(
        output_dir: &Path,
        completion: CompletionEvidence,
    ) -> Result<Self, HistoryError> {
        completion.validate()?;
        let artifacts = GraphArtifacts::load_with_registry_mode(output_dir, &[], false)?;
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
    /// Construct history artifacts from an already validated typed graph.
    ///
    /// This is the in-process counterpart of loading `graph.json`; it retains
    /// the same compatibility projection and canonical authoritative bytes
    /// without serializing and deserializing through the filesystem boundary.
    #[doc(hidden)]
    pub fn from_trusted(
        trusted: TrustedGraphDocument,
        program: Option<AnalysisBundle>,
        analysis: Option<Value>,
        manifest: Option<Value>,
    ) -> Result<Self, HistoryError> {
        let trusted_bytes = canonical_trusted_graph_bytes(&trusted)?;
        let graph = serde_json::to_value(&trusted.graph)?
            .as_object()
            .cloned()
            .unwrap_or_default();
        let nodes = trusted
            .nodes
            .iter()
            .map(compat_node)
            .collect::<Result<Vec<_>, _>>()?;
        let links = trusted
            .links
            .iter()
            .map(compat_edge)
            .collect::<Result<Vec<_>, _>>()?;
        Ok(Self {
            document: GraphDocument {
                directed: trusted.directed,
                multigraph: trusted.multigraph,
                graph,
                nodes,
                links,
                extras: BTreeMap::new(),
            },
            program,
            analysis,
            labels: None,
            manifest,
            authoritative_sidecars: BTreeMap::from([(
                TRUSTED_GRAPH_CONTENT.to_owned(),
                trusted_bytes,
            )]),
        })
    }

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
            .filter(|(path, _)| !is_internal_artifact(path))
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
        Self::load_with_registry_mode(output_dir, registry, true)
    }

    fn load_with_registry_mode(
        output_dir: &Path,
        registry: &[ArtifactRegistryEntry],
        validate_program: bool,
    ) -> Result<Self, HistoryError> {
        let total_started = Instant::now();
        validate_registry_declarations(registry)?;
        let mut authoritative_sidecars = BTreeMap::new();
        let sidecars_started = Instant::now();
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
        profile_artifact_load("authoritative sidecars", sidecars_started);
        let graph_path = output_dir.join("graph.json");
        let program_path = output_dir.join("program.json");
        let ((document, trusted_graph), program) = {
            let (graph, program) = rayon::join(
                || {
                    let started = Instant::now();
                    let result = load_trusted_graph(&graph_path);
                    profile_artifact_load("trusted graph", started);
                    result
                },
                || {
                    let started = Instant::now();
                    let result = read_optional_program(&program_path, validate_program);
                    profile_artifact_load("Program", started);
                    result
                },
            );
            // Preserve stable error precedence even though both operations run
            // to completion concurrently.
            (graph?, program?)
        };
        authoritative_sidecars.insert(TRUSTED_GRAPH_CONTENT.to_owned(), trusted_graph);
        if !validate_program && let Some(digest) = program.source_digest {
            authoritative_sidecars
                .insert(PROGRAM_SOURCE_DIGEST_CONTENT.to_owned(), digest.to_vec());
        }
        let json_sidecars_started = Instant::now();
        let artifacts = Self {
            document,
            program: program.bundle,
            analysis: read_optional_json(&output_dir.join("analysis.json"))?,
            labels: read_optional_json(&output_dir.join("labels.json"))?,
            manifest: read_optional_json(&output_dir.join("manifest.json"))?,
            authoritative_sidecars,
        };
        profile_artifact_load("JSON sidecars", json_sidecars_started);
        let registry_started = Instant::now();
        verify_builtin_registry_content(&artifacts, registry)?;
        profile_artifact_load("artifact registry", registry_started);
        profile_artifact_load("total", total_started);
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
            .contains_key(TRUSTED_GRAPH_CONTENT);
        if !trusted_graph {
            canonicalize_graph_document(&mut self.document)?;
        }
        let program_bytes = self
            .program
            .as_ref()
            .map(AnalysisBundle::canonical_bytes)
            .transpose()?;
        if let (Some(bytes), Some(source_digest)) = (
            program_bytes.as_deref(),
            self.authoritative_sidecars
                .get(PROGRAM_SOURCE_DIGEST_CONTENT),
        ) {
            let canonical_digest: [u8; 32] = Sha256::digest(bytes).into();
            if source_digest.as_slice() != canonical_digest {
                return Err(HistoryError::InvalidArtifacts(
                    "program.json is not canonical".to_owned(),
                ));
            }
        }
        let registry = artifact_registry_from_canonical(&self, program_bytes.as_deref())?;
        let mut partitioned = PartitionedGraph::default();

        if let Some(program) = self.program.take() {
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
            partitioned.program_facts.extend(
                providers
                    .into_par_iter()
                    .map(|provider| {
                        Ok((
                            program_key("provider", &provider.id),
                            encode_record(
                                "compass.program.provider",
                                &serde_json::to_value(provider)?,
                            )?,
                        ))
                    })
                    .collect::<Result<Vec<_>, HistoryError>>()?,
            );
            partitioned.program_facts.extend(
                evidence
                    .into_par_iter()
                    .map(|evidence| {
                        Ok((
                            program_key("evidence", &evidence.id),
                            encode_record(
                                "compass.program.evidence",
                                &serde_json::to_value(evidence)?,
                            )?,
                        ))
                    })
                    .collect::<Result<Vec<_>, HistoryError>>()?,
            );
            let module_records = modules
                .into_par_iter()
                .map(|mut module| {
                    let key = program_key("module", &module.source_file);
                    let function_ids = module
                        .functions
                        .iter()
                        .map(|function| function.symbol_id.clone())
                        .collect();
                    let functions = std::mem::take(&mut module.functions);
                    let module_record = (
                        key,
                        encode_record(
                            "compass.program.module-index",
                            &serde_json::to_value(IndexedProgramModule {
                                module,
                                function_ids,
                            })?,
                        )?,
                    );
                    let functions = functions
                        .into_iter()
                        .map(|function| {
                            Ok((
                                program_key("function", &function.symbol_id),
                                encode_record(
                                    "compass.program.function",
                                    &serde_json::to_value(function)?,
                                )?,
                            ))
                        })
                        .collect::<Result<Vec<_>, HistoryError>>()?;
                    Ok((module_record, functions))
                })
                .collect::<Result<Vec<_>, HistoryError>>()?;
            for (module, functions) in module_records {
                partitioned.program_facts.push(module);
                partitioned.program_facts.extend(functions);
            }
            partitioned.program_summaries.extend(
                summaries
                    .into_par_iter()
                    .map(|summary| {
                        Ok((
                            program_key("summary", &summary.symbol_id),
                            encode_record(
                                "compass.program.summary",
                                &serde_json::to_value(summary)?,
                            )?,
                        ))
                    })
                    .collect::<Result<Vec<_>, HistoryError>>()?,
            );
            partitioned.program_summaries.extend(
                reverse_calls
                    .into_par_iter()
                    .map(|(target, callers)| {
                        Ok((
                            program_key("reverse-call", &target),
                            encode_record(
                                "compass.program.reverse-call",
                                &serde_json::to_value(callers)?,
                            )?,
                        ))
                    })
                    .collect::<Result<Vec<_>, HistoryError>>()?,
            );
        }

        // Strict v1 input was validated in canonical contract order when it
        // was loaded, so records can consume that order without retaining a
        // second graph-sized ordering index.
        if trusted_graph {
            partitioned.nodes = self
                .document
                .nodes
                .par_iter()
                .map(|node| {
                    Ok((
                        node_key(&node.id),
                        encode_record("compass.graph.node.v1", &trusted_node_value(node))?,
                    ))
                })
                .collect::<Result<Vec<_>, HistoryError>>()?;
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
        if trusted_graph {
            partitioned.edges = self
                .document
                .links
                .par_iter()
                .map(|edge| {
                    let id = edge
                        .attributes
                        .get("id")
                        .and_then(Value::as_str)
                        .ok_or_else(|| {
                            HistoryError::InvalidArtifacts(
                                "trusted edge has no string id".to_owned(),
                            )
                        })?;
                    let kind = edge
                        .attributes
                        .get("kind")
                        .and_then(Value::as_str)
                        .ok_or_else(|| {
                            HistoryError::InvalidArtifacts(
                                "trusted edge has no string kind".to_owned(),
                            )
                        })?;
                    Ok((
                        edge_key(&edge.source, &edge.target, kind, true, Some(id.as_bytes())),
                        encode_record("compass.graph.edge.v1", &trusted_edge_value(edge))?,
                    ))
                })
                .collect::<Result<Vec<_>, HistoryError>>()?;
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
        if trusted_graph {
            partitioned.metadata.push((
                metadata_key(&[b"trusted-graph"]),
                encode_record(
                    "compass.metadata.trusted-graph",
                    &serde_json::to_value(TrustedGraphMarker {
                        schema: compass_model::code_graph::CODE_GRAPH_SCHEMA_V1.to_owned(),
                    })?,
                )?,
            ));
        }

        add_optional_analysis(&mut partitioned, "analysis.json", self.analysis.take())?;
        add_optional_analysis(&mut partitioned, "labels.json", self.labels.take())?;
        if let Some(manifest) = self.manifest.take() {
            let manifest = canonical_manifest_owned(manifest);
            partitioned.metadata.push((
                metadata_key(&[b"manifest"]),
                encode_record("compass.metadata.manifest", &manifest)?,
            ));
        }
        for (path, bytes) in std::mem::take(&mut self.authoritative_sidecars) {
            if is_internal_artifact(&path) {
                continue;
            }
            if path == SOURCE_INVENTORY_CONTENT {
                let inventory = serde_json::from_slice::<Value>(&bytes)?;
                if canonical_json_bytes(&inventory)? != bytes {
                    return Err(HistoryError::InvalidArtifacts(
                        "source inventory is not canonical JSON".to_owned(),
                    ));
                }
                partitioned.metadata.push((
                    metadata_key(&[b"source-inventory"]),
                    encode_record("compass.metadata.source-inventory", &inventory)?,
                ));
                continue;
            }
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
                        b"analysis.json" => analysis = Some(value),
                        b"labels.json" => labels = Some(value),
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
        let mut header = None;
        let mut completion = None;
        let mut registry = None;
        let mut manifest = None;
        let mut sidecars = BTreeMap::new();
        let mut trusted_graph_marker = None;
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
                [_, _, name] if name == b"trusted-graph" => {
                    let marker: TrustedGraphMarker =
                        decode_typed(bytes, "compass.metadata.trusted-graph")?;
                    if marker.schema != compass_model::code_graph::CODE_GRAPH_SCHEMA_V1
                        || trusted_graph_marker.replace(marker).is_some()
                    {
                        return Err(HistoryError::InvalidArtifacts(
                            "invalid trusted graph marker".to_owned(),
                        ));
                    }
                }
                [_, _, name] if name == b"source-inventory" => {
                    let inventory = decode_record(bytes, "compass.metadata.source-inventory")?;
                    if sidecars
                        .insert(
                            SOURCE_INVENTORY_CONTENT.to_owned(),
                            canonical_json_bytes(&inventory)?,
                        )
                        .is_some()
                    {
                        return Err(HistoryError::InvalidArtifacts(
                            "duplicate source inventory".to_owned(),
                        ));
                    }
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
        let (mut ordered_nodes, trusted_nodes) = if trusted_graph_marker.is_some() {
            let mut nodes = decode_trusted_node_map(&partitioned.nodes)?;
            let mut trusted = if node_order.is_empty() {
                nodes.into_values().collect::<Vec<_>>()
            } else {
                restore_order(&mut nodes, node_order, "node")?
            };
            sort_trusted_nodes(&mut trusted);
            let compatible = trusted
                .iter()
                .map(compat_node)
                .collect::<Result<Vec<_>, _>>()?;
            (compatible, Some(trusted))
        } else {
            let mut nodes = decode_node_map(&partitioned.nodes)?;
            (restore_order(&mut nodes, node_order, "node")?, None)
        };
        let (ordered_edges, trusted_edges) = if trusted_graph_marker.is_some() {
            let mut edges = decode_trusted_edge_map(&partitioned.edges)?;
            let mut trusted = if edge_order.is_empty() {
                edges.into_values().collect::<Vec<_>>()
            } else {
                restore_order(&mut edges, edge_order, "edge")?
            };
            sort_trusted_edges(&mut trusted);
            let compatible = trusted
                .iter()
                .map(compat_edge)
                .collect::<Result<Vec<_>, _>>()?;
            (compatible, Some(trusted))
        } else {
            let mut edges = decode_edge_map(&partitioned.edges)?;
            (restore_order(&mut edges, edge_order, "edge")?, None)
        };
        for node in &mut ordered_nodes {
            if let Some(fields) = node_analysis.remove(&node.id) {
                node.attributes.extend(fields);
            }
        }
        if !node_analysis.is_empty() {
            return Err(HistoryError::InvalidArtifacts(
                "analysis references a missing node".to_owned(),
            ));
        }
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
        if let (Some(nodes), Some(links)) = (trusted_nodes, trusted_edges) {
            if header.graph_hyperedges_present
                || header.top_hyperedges_present
                || !header.extras.is_empty()
            {
                return Err(HistoryError::InvalidArtifacts(
                    "trusted graph metadata contains unsupported extension fields".to_owned(),
                ));
            }
            let graph = serde_json::from_value(Value::Object(header.graph.clone()))?;
            let trusted = TrustedGraphDocument {
                directed: header.directed,
                multigraph: header.multigraph,
                graph,
                nodes,
                links,
            };
            sidecars.insert(
                TRUSTED_GRAPH_CONTENT.to_owned(),
                canonical_json_bytes(&serde_json::to_value(trusted)?)?,
            );
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
            write_json_atomic(output_dir.join("analysis.json"), value, false)?;
        }
        if let Some(value) = &self.labels {
            write_json_atomic(output_dir.join("labels.json"), value, false)?;
        }
        if let Some(value) = &self.manifest {
            write_json_atomic(output_dir.join("manifest.json"), value, false)?;
        }
        for (path, bytes) in &self.authoritative_sidecars {
            if is_internal_artifact(path) {
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
            output_dir.join("semantic-marker.json"),
            &SemanticCompletionMarker::from(completion),
            false,
        )?;
        Ok(())
    }
}

fn load_trusted_graph(path: &Path) -> Result<(GraphDocument, Vec<u8>), HistoryError> {
    let trusted = TrustedGraphDocument::load_for_recluster(path)?;
    let trusted_bytes = canonical_trusted_graph_bytes(&trusted)?;
    let graph = serde_json::to_value(&trusted.graph)?
        .as_object()
        .cloned()
        .unwrap_or_default();
    let nodes = trusted
        .nodes
        .iter()
        .map(compat_node)
        .collect::<Result<Vec<_>, _>>()?;
    let links = trusted
        .links
        .iter()
        .map(compat_edge)
        .collect::<Result<Vec<_>, _>>()?;
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

fn canonical_trusted_graph_bytes(trusted: &TrustedGraphDocument) -> Result<Vec<u8>, HistoryError> {
    let mut canonical = trusted.clone();
    sort_trusted_nodes(&mut canonical.nodes);
    sort_trusted_edges(&mut canonical.links);
    canonical_json_bytes(&serde_json::to_value(canonical)?)
}

fn sort_trusted_nodes(nodes: &mut [compass_model::code_graph::NodeRecord]) {
    nodes.sort_by(|left, right| left.id.cmp(&right.id));
}

fn sort_trusted_edges(edges: &mut [compass_model::code_graph::EdgeRecord]) {
    edges.sort_by(|left, right| {
        (
            left.source.as_str(),
            left.kind.as_str(),
            left.target.as_str(),
            left.key.as_str(),
        )
            .cmp(&(
                right.source.as_str(),
                right.kind.as_str(),
                right.target.as_str(),
                right.key.as_str(),
            ))
    });
}

fn compat_node(node: &compass_model::code_graph::NodeRecord) -> Result<NodeRecord, HistoryError> {
    let mut object = serde_json::to_value(node)?
        .as_object()
        .cloned()
        .ok_or_else(|| {
            HistoryError::InvalidArtifacts("trusted node is not an object".to_owned())
        })?;
    let id = object
        .remove("id")
        .and_then(|value| value.as_str().map(str::to_owned))
        .ok_or_else(|| {
            HistoryError::InvalidArtifacts("trusted node has no string id".to_owned())
        })?;
    for (key, value) in node.properties().filter(|(key, _)| *key != "id") {
        object.entry(key.to_owned()).or_insert(value);
    }
    Ok(NodeRecord {
        id,
        attributes: object,
    })
}

fn compat_edge(edge: &compass_model::code_graph::EdgeRecord) -> Result<EdgeRecord, HistoryError> {
    let mut object = serde_json::to_value(edge)?
        .as_object()
        .cloned()
        .ok_or_else(|| {
            HistoryError::InvalidArtifacts("trusted edge is not an object".to_owned())
        })?;
    let source = object
        .remove("source")
        .and_then(|value| value.as_str().map(str::to_owned))
        .ok_or_else(|| {
            HistoryError::InvalidArtifacts("trusted edge has no string source".to_owned())
        })?;
    let target = object
        .remove("target")
        .and_then(|value| value.as_str().map(str::to_owned))
        .ok_or_else(|| {
            HistoryError::InvalidArtifacts("trusted edge has no string target".to_owned())
        })?;
    for (key, value) in edge
        .properties()
        .filter(|(key, _)| !matches!(*key, "source" | "target"))
    {
        object.entry(key.to_owned()).or_insert(value);
    }
    Ok(EdgeRecord {
        source,
        target,
        attributes: object,
    })
}

fn trusted_node_value(node: &NodeRecord) -> Value {
    let mut object = node.attributes.clone();
    for field in NODE_COMPATIBILITY_FIELDS {
        object.remove(field);
    }
    object.insert("id".to_owned(), Value::String(node.id.clone()));
    Value::Object(object)
}

fn trusted_edge_value(edge: &EdgeRecord) -> Value {
    let mut object = edge.attributes.clone();
    for field in EDGE_COMPATIBILITY_FIELDS {
        object.remove(field);
    }
    object.insert("source".to_owned(), Value::String(edge.source.clone()));
    object.insert("target".to_owned(), Value::String(edge.target.clone()));
    Value::Object(object)
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
    artifact_registry_with_graph_bytes(artifacts, &graph_bytes, None)
}

fn artifact_registry_from_canonical(
    artifacts: &GraphArtifacts,
    program_bytes: Option<&[u8]>,
) -> Result<Vec<ArtifactRegistryEntry>, HistoryError> {
    if let Some(graph_bytes) = artifacts.authoritative_sidecars.get(TRUSTED_GRAPH_CONTENT) {
        artifact_registry_with_graph_bytes(artifacts, graph_bytes, program_bytes)
    } else {
        let graph_bytes = canonical_json_bytes(&serde_json::to_value(&artifacts.document)?)?;
        artifact_registry_with_graph_bytes(artifacts, &graph_bytes, program_bytes)
    }
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
    program_bytes: Option<&[u8]>,
) -> Result<Vec<ArtifactRegistryEntry>, HistoryError> {
    let mut registry = vec![authoritative_entry(
        "graph.json",
        "application/json",
        graph_bytes,
    )];
    if let Some(program) = &artifacts.program {
        let canonical;
        let bytes = if let Some(program_bytes) = program_bytes {
            program_bytes
        } else {
            canonical = program.canonical_bytes()?;
            &canonical
        };
        registry.push(authoritative_entry(
            "program.json",
            "application/json",
            bytes,
        ));
    }
    for (path, value) in [
        ("analysis.json", artifacts.analysis.as_ref()),
        ("labels.json", artifacts.labels.as_ref()),
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
        if is_internal_artifact(path) {
            continue;
        }
        let mut entry = authoritative_entry(path, "application/octet-stream", bytes);
        if path != SOURCE_INVENTORY_CONTENT {
            entry.storage = Some(bytes.clone());
        }
        registry.push(entry);
    }
    for path in [
        "GRAPH_REPORT.md",
        "graph.html",
        "GRAPH_TREE.html",
        "labels.json.sig",
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
        relative_path: "semantic-marker.json".to_owned(),
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
        "graph.json" | "program.json" | "analysis.json" | "labels.json" | "manifest.json"
    )
}

fn is_internal_artifact(path: &str) -> bool {
    matches!(path, TRUSTED_GRAPH_CONTENT | PROGRAM_SOURCE_DIGEST_CONTENT)
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
            "analysis.json" => artifacts
                .analysis
                .as_ref()
                .map(canonical_json_bytes)
                .transpose()?,
            "labels.json" => artifacts
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

pub(crate) fn metadata_key(parts: &[&[u8]]) -> Vec<u8> {
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
    let mut modules = Vec::<DecodedProgramModule>::new();
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
                let value = decode_program_module(bytes)?;
                let source_file = match &value {
                    DecodedProgramModule::Embedded(module) => &module.source_file,
                    DecodedProgramModule::Indexed(indexed) => &indexed.module.source_file,
                };
                if source_file != identity {
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
    let mut referenced_functions = BTreeSet::new();
    let modules = modules
        .into_iter()
        .map(|stored| match stored {
            DecodedProgramModule::Embedded(module) => {
                for function in &module.functions {
                    let indexed = indexed_functions.get(&function.symbol_id).ok_or_else(|| {
                        HistoryError::InvalidArtifacts(
                            "program module references a missing indexed function".to_owned(),
                        )
                    })?;
                    if indexed != function
                        || !referenced_functions.insert(function.symbol_id.clone())
                    {
                        return Err(HistoryError::InvalidArtifacts(
                            "indexed program functions do not match module contents".to_owned(),
                        ));
                    }
                }
                Ok(module)
            }
            DecodedProgramModule::Indexed(mut indexed) => {
                indexed.module.functions = indexed
                    .function_ids
                    .into_iter()
                    .map(|symbol_id| {
                        if !referenced_functions.insert(symbol_id.clone()) {
                            return Err(HistoryError::InvalidArtifacts(
                                "program function is indexed by multiple modules".to_owned(),
                            ));
                        }
                        indexed_functions.get(&symbol_id).cloned().ok_or_else(|| {
                            HistoryError::InvalidArtifacts(
                                "program module references a missing indexed function".to_owned(),
                            )
                        })
                    })
                    .collect::<Result<Vec<_>, _>>()?;
                Ok(indexed.module)
            }
        })
        .collect::<Result<Vec<_>, HistoryError>>()?;
    if referenced_functions.len() != indexed_functions.len() {
        return Err(HistoryError::InvalidArtifacts(
            "unreferenced indexed program function".to_owned(),
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

pub(crate) fn decode_program_module(bytes: &[u8]) -> Result<DecodedProgramModule, HistoryError> {
    let envelope = VersionedValue::from_bytes(bytes)?;
    envelope.require_schema(&envelope.schema, RECORD_VERSION)?;
    match envelope.schema.as_str() {
        "compass.program.module" => Ok(DecodedProgramModule::Embedded(serde_json::from_slice(
            &envelope.payload,
        )?)),
        "compass.program.module-index" => Ok(DecodedProgramModule::Indexed(
            serde_json::from_slice(&envelope.payload)?,
        )),
        schema => Err(HistoryError::InvalidArtifacts(format!(
            "unsupported program module record schema {schema}"
        ))),
    }
}

fn decode_node_map(
    entries: &[(Vec<u8>, Vec<u8>)],
) -> Result<BTreeMap<Vec<u8>, NodeRecord>, HistoryError> {
    entries
        .iter()
        .map(|(key, bytes)| Ok((key.clone(), decode_compatible_node(bytes)?)))
        .collect()
}

fn decode_trusted_node_map(
    entries: &[(Vec<u8>, Vec<u8>)],
) -> Result<BTreeMap<Vec<u8>, compass_model::code_graph::NodeRecord>, HistoryError> {
    entries
        .iter()
        .map(|(key, bytes)| {
            let envelope = VersionedValue::from_bytes(bytes)?;
            envelope.require_schema("compass.graph.node.v1", RECORD_VERSION)?;
            Ok((key.clone(), serde_json::from_slice(&envelope.payload)?))
        })
        .collect()
}

pub(crate) fn decode_compatible_node(bytes: &[u8]) -> Result<NodeRecord, HistoryError> {
    let envelope = VersionedValue::from_bytes(bytes)?;
    match envelope.schema.as_str() {
        "compass.node" => {
            envelope.require_schema("compass.node", RECORD_VERSION)?;
            serde_json::from_slice(&envelope.payload).map_err(HistoryError::from)
        }
        "compass.graph.node.v1" => {
            envelope.require_schema("compass.graph.node.v1", RECORD_VERSION)?;
            let typed =
                serde_json::from_slice::<compass_model::code_graph::NodeRecord>(&envelope.payload)?;
            Ok(compat_node(&typed)?)
        }
        schema => Err(HistoryError::InvalidArtifacts(format!(
            "unexpected node record schema {schema}"
        ))),
    }
}

fn decode_edge_map(
    entries: &[(Vec<u8>, Vec<u8>)],
) -> Result<BTreeMap<Vec<u8>, EdgeRecord>, HistoryError> {
    entries
        .iter()
        .map(|(key, bytes)| Ok((key.clone(), decode_compatible_edge(bytes)?)))
        .collect()
}

fn decode_trusted_edge_map(
    entries: &[(Vec<u8>, Vec<u8>)],
) -> Result<BTreeMap<Vec<u8>, compass_model::code_graph::EdgeRecord>, HistoryError> {
    entries
        .iter()
        .map(|(key, bytes)| {
            let envelope = VersionedValue::from_bytes(bytes)?;
            envelope.require_schema("compass.graph.edge.v1", RECORD_VERSION)?;
            Ok((key.clone(), serde_json::from_slice(&envelope.payload)?))
        })
        .collect()
}

pub(crate) fn decode_compatible_edge(bytes: &[u8]) -> Result<EdgeRecord, HistoryError> {
    let envelope = VersionedValue::from_bytes(bytes)?;
    match envelope.schema.as_str() {
        "compass.edge" => {
            envelope.require_schema("compass.edge", RECORD_VERSION)?;
            serde_json::from_slice(&envelope.payload).map_err(HistoryError::from)
        }
        "compass.graph.edge.v1" => {
            envelope.require_schema("compass.graph.edge.v1", RECORD_VERSION)?;
            let typed =
                serde_json::from_slice::<compass_model::code_graph::EdgeRecord>(&envelope.payload)?;
            Ok(compat_edge(&typed)?)
        }
        schema => Err(HistoryError::InvalidArtifacts(format!(
            "unexpected edge record schema {schema}"
        ))),
    }
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

fn read_optional_program(
    path: &Path,
    validate_canonical: bool,
) -> Result<LoadedProgram, HistoryError> {
    let read_started = Instant::now();
    let bytes = match fs::read(path) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Ok(LoadedProgram {
                bundle: None,
                source_digest: None,
            });
        }
        Err(source) => return Err(crate::error::io_error(path, source)),
    };
    profile_artifact_load("Program read", read_started);
    let decode_started = Instant::now();
    let program: AnalysisBundle = serde_json::from_slice(&bytes)?;
    profile_artifact_load("Program deserialize", decode_started);
    if validate_canonical {
        let canonical_started = Instant::now();
        let canonical = program.canonical_bytes()?;
        profile_artifact_load("Program canonical validation", canonical_started);
        if canonical != bytes {
            return Err(HistoryError::InvalidArtifacts(
                "program.json is not canonical".to_owned(),
            ));
        }
    }
    Ok(LoadedProgram {
        bundle: Some(program),
        source_digest: Some(Sha256::digest(&bytes).into()),
    })
}

fn profile_artifact_load(name: &str, started: Instant) {
    if std::env::var_os("COMPASS_PROFILE_INTERNAL").is_some() {
        eprintln!(
            "[compass history artifact load] {name}: {:.3}s",
            started.elapsed().as_secs_f64()
        );
    }
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
        assert!(is_builtin_artifact("analysis.json"));
        assert!(is_builtin_artifact("labels.json"));
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
