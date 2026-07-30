use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};

use compass_languages::{Extraction, RawEdgeRecord, RawNodeRecord, Registry};
use compass_model::code_graph::{
    BuildMetadata, CommunityMetadata, ConfigNodeDetails, CoverageRecord, CoverageStatus,
    DatabaseNodeDetails, DiagnosticSeverity, EdgeDetails, EdgeKind, EdgeRecord, ExtractionStatus,
    FileNodeDetails, FileRecord, GraphDiagnostic, GraphDocument, GraphMetadata,
    ImportExportNodeDetails, MessagingNodeDetails, NodeDetails, NodeKind, NodeRecord, NodeRole,
    QueryNodeDetails, ResourceKind, ResourceNodeDetails, RouteEdgeDetails, RouteNodeDetails,
    RouteStage, RouteStageDetails, SchemaNodeDetails, SymbolNodeDetails,
};
use compass_model::identity::{
    database_entity_id, domain_id, edge_id, file_id, messaging_id, normalize_repository_path,
    route_id, symbol_id,
};
use compass_model::provenance::{
    COALESCED_NODE_EVIDENCE_ATTRIBUTE, CONSUME_INCREMENTAL_ENDPOINT_REMAP_ATTRIBUTE,
    ENDPOINT_REWRITE_RULES_ATTRIBUTE, EndpointRewriteRule, EvidenceConfidence, EvidenceOrigin,
    OCCURRENCE_RULE_ATTRIBUTE, OccurrenceRule, Provenance, ResolutionCandidate, ResolutionState,
    SEMANTIC_LAYER_EXTRACTOR, SourceAnchor, TRUSTED_EDGE_RECORD_ATTRIBUTE,
    TRUSTED_NODE_RECORD_ATTRIBUTE,
};
use compass_model::{
    CodeGraphValidationError, GraphError, validate_code_graph, validate_code_graph_records,
};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use crate::quarantine::{PublicationOutcome, QuarantineCollector};

/// Version of the normalization and publication contract behind graph schema v1.
pub const V1_PUBLICATION_SEMANTICS_VERSION: &str = "compass.graph.publication/2";
use sha2::{Digest, Sha256};

const TRUSTED_NODE_RECORD: &str = TRUSTED_NODE_RECORD_ATTRIBUTE;
const TRUSTED_EDGE_RECORD: &str = TRUSTED_EDGE_RECORD_ATTRIBUTE;
const TRUSTED_GRAPH_COVERAGE: &str = "_compass_v1_graph_coverage";
const TRUSTED_GRAPH_DIAGNOSTICS: &str = "_compass_v1_graph_diagnostics";
const COALESCED_EDGE_EVIDENCE: &str = "_coalesced_edge_evidence";
const MAX_EXTERNAL_REFERENCE_DIAGNOSTICS: usize = 100;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum PublicationMode {
    Strict,
    BestEffort,
}

#[derive(Clone, Debug)]
pub struct BuildEvidence {
    pub repository_root: PathBuf,
    pub build: BuildMetadata,
    pub files: Vec<FileRecord>,
    pub coverage: Vec<CoverageRecord>,
    pub diagnostics: Vec<GraphDiagnostic>,
}

#[derive(Clone, Debug)]
pub struct InventoryEvidence {
    pub path: PathBuf,
    pub language: Option<String>,
    pub producer: String,
    pub status: ExtractionStatus,
    pub reason: Option<String>,
}

impl BuildEvidence {
    #[must_use]
    pub fn new(repository_root: impl Into<PathBuf>, build: BuildMetadata) -> Self {
        Self {
            repository_root: repository_root.into(),
            build,
            files: Vec::new(),
            coverage: Vec::new(),
            diagnostics: Vec::new(),
        }
    }

    /// Derive deterministic build and file evidence from resolved raw facts.
    pub fn from_extraction(
        repository_root: impl Into<PathBuf>,
        extraction: &Extraction,
        configuration_digest: impl Into<String>,
    ) -> Result<Self, GraphError> {
        let repository_root = repository_root.into();
        let mut paths = BTreeSet::new();
        let mut external_reference_anchors = BTreeMap::<String, SourceAnchor>::new();
        let mut diagnostics = Vec::new();
        for attributes in extraction
            .nodes
            .iter()
            .map(|node| &node.attributes)
            .chain(extraction.edges.iter().map(|edge| &edge.attributes))
        {
            if let Some((path, anchor)) = external_reference_anchor(attributes, &repository_root)? {
                external_reference_anchors
                    .entry(path)
                    .and_modify(|existing| {
                        if anchor_key(&anchor) < anchor_key(existing) {
                            existing.clone_from(&anchor);
                        }
                    })
                    .or_insert(anchor);
            }
            for path in ["source_file", "origin_file"]
                .into_iter()
                .filter_map(|key| optional_source_path(attributes, key))
            {
                paths.insert(portable_path(&path, &repository_root)?);
            }
        }

        let mut source_tree = Sha256::new();
        let mut files = Vec::with_capacity(paths.len());
        let mut omitted_external_references = 0_usize;
        for path in paths {
            let absolute = repository_root.join(&path);
            if !absolute.is_file() {
                if diagnostics.len() < MAX_EXTERNAL_REFERENCE_DIAGNOSTICS {
                    evidence_external_reference_diagnostic(
                        &mut diagnostics,
                        &path,
                        external_reference_anchors.get(&path).cloned(),
                    );
                } else {
                    omitted_external_references = omitted_external_references.saturating_add(1);
                }
                continue;
            }
            let language = Registry::resolve(&absolute).map(|spec| spec.name.to_owned());
            let bytes = fs::read(&absolute).map_err(|source| GraphError::Read {
                path: absolute,
                source,
            })?;
            let content_digest = sha256_prefixed(&bytes);
            source_tree.update((path.len() as u64).to_le_bytes());
            source_tree.update(path.as_bytes());
            source_tree.update((content_digest.len() as u64).to_le_bytes());
            source_tree.update(content_digest.as_bytes());
            files.push(FileRecord {
                id: file_id(&path),
                path,
                language,
                content_digest,
                byte_size: bytes.len() as u64,
                generated: false,
                extraction_status: ExtractionStatus::Extracted,
                extractor_versions: vec![format!(
                    "compass-languages/{}",
                    env!("CARGO_PKG_VERSION")
                )],
                coverage: Vec::new(),
                diagnostics: Vec::new(),
            });
        }
        if omitted_external_references > 0 {
            diagnostics.push(GraphDiagnostic {
                severity: DiagnosticSeverity::Warning,
                code: "unresolved_external_reference_truncated".to_owned(),
                message: format!(
                    "omitted {omitted_external_references} additional unresolved external references"
                ),
                anchor: None,
                related_ids: Vec::new(),
            });
        }

        let configuration_digest = configuration_digest.into();
        let source_tree_digest = format!("sha256:{:x}", source_tree.finalize());
        let schema_fingerprint = schema_fingerprint();
        let mut generation = Sha256::new();
        for value in [
            compass_model::code_graph::CODE_GRAPH_SCHEMA_V1,
            env!("CARGO_PKG_VERSION"),
            &schema_fingerprint,
            &source_tree_digest,
            &configuration_digest,
        ] {
            generation.update((value.len() as u64).to_le_bytes());
            generation.update(value.as_bytes());
        }
        let generation_id = format!("sha256:{:x}", generation.finalize());
        let mut declared_coverage = BTreeSet::new();
        for node in &extraction.nodes {
            if let Some(kind) =
                optional_any_string(&node.attributes, &["symbol_kind", "type", "file_type"])
            {
                declared_coverage.insert((
                    format!("node:{kind}"),
                    coverage_producer(&node.attributes),
                    coverage_file_id(&node.attributes, &repository_root)?,
                ));
            }
        }
        for edge in &extraction.edges {
            if let Some(relation) = optional_string(&edge.attributes, "relation") {
                declared_coverage.insert((
                    format!("edge:{relation}"),
                    coverage_producer(&edge.attributes),
                    coverage_file_id(&edge.attributes, &repository_root)?,
                ));
            }
        }
        let coverage = declared_coverage
            .into_iter()
            .map(|(capability, producer, file_id)| CoverageRecord {
                capability,
                producer,
                status: CoverageStatus::Complete,
                file_id,
                reason: None,
                anchor: None,
            })
            .collect();
        Ok(Self {
            repository_root,
            build: BuildMetadata {
                builder_version: env!("CARGO_PKG_VERSION").to_owned(),
                schema_fingerprint,
                source_tree_digest,
                configuration_digest,
                generation_id,
                source_commit: None,
            },
            files,
            coverage,
            diagnostics,
        })
    }

    pub fn include_inventory(
        &mut self,
        inventory: impl IntoIterator<Item = InventoryEvidence>,
    ) -> Result<(), GraphError> {
        for item in inventory {
            let path = portable_path(&item.path.to_string_lossy(), &self.repository_root)?;
            let absolute = self.repository_root.join(&path);
            if !absolute.is_file() {
                continue;
            }
            let bytes = fs::read(&absolute).map_err(|source| GraphError::Read {
                path: absolute,
                source,
            })?;
            let record = FileRecord {
                id: file_id(&path),
                path: path.clone(),
                language: item.language.clone(),
                content_digest: sha256_prefixed(&bytes),
                byte_size: bytes.len() as u64,
                generated: item.status == ExtractionStatus::Generated,
                extraction_status: item.status,
                extractor_versions: vec![format!(
                    "compass-languages/{}",
                    env!("CARGO_PKG_VERSION")
                )],
                coverage: Vec::new(),
                diagnostics: inventory_diagnostic(&path, item.status, item.reason.as_deref())
                    .into_iter()
                    .collect(),
            };
            if let Some(existing) = self.files.iter_mut().find(|file| file.path == path) {
                *existing = record;
            } else {
                self.files.push(record);
            }
            let status = coverage_status(item.status);
            let file_record_id = file_id(&path);
            for coverage in &mut self.coverage {
                if coverage.file_id.as_deref() == Some(file_record_id.as_str())
                    && status != CoverageStatus::Complete
                {
                    coverage.status = status;
                    coverage.reason.clone_from(&item.reason);
                }
            }
            self.coverage.push(CoverageRecord {
                capability: "file_inventory".to_owned(),
                producer: item.producer,
                status,
                file_id: Some(file_record_id),
                reason: item.reason.clone(),
                anchor: None,
            });
            if let Some(diagnostic) =
                inventory_diagnostic(&path, item.status, item.reason.as_deref())
            {
                self.diagnostics.push(diagnostic);
            }
        }
        self.files.sort_by(|left, right| left.path.cmp(&right.path));
        let mut source_tree = Sha256::new();
        for file in &self.files {
            source_tree.update((file.path.len() as u64).to_le_bytes());
            source_tree.update(file.path.as_bytes());
            source_tree.update((file.content_digest.len() as u64).to_le_bytes());
            source_tree.update(file.content_digest.as_bytes());
        }
        self.build.source_tree_digest = format!("sha256:{:x}", source_tree.finalize());
        let mut generation = Sha256::new();
        for value in [
            compass_model::code_graph::CODE_GRAPH_SCHEMA_V1,
            self.build.builder_version.as_str(),
            self.build.schema_fingerprint.as_str(),
            self.build.source_tree_digest.as_str(),
            self.build.configuration_digest.as_str(),
        ] {
            generation.update((value.len() as u64).to_le_bytes());
            generation.update(value.as_bytes());
        }
        self.build.generation_id = format!("sha256:{:x}", generation.finalize());
        Ok(())
    }
}

fn coverage_status(status: ExtractionStatus) -> CoverageStatus {
    match status {
        ExtractionStatus::Extracted => CoverageStatus::Complete,
        ExtractionStatus::Partial => CoverageStatus::Partial,
        ExtractionStatus::Unsupported => CoverageStatus::Unsupported,
        ExtractionStatus::Excluded => CoverageStatus::Excluded,
        ExtractionStatus::ParseFailure => CoverageStatus::Failed,
        ExtractionStatus::Generated | ExtractionStatus::Binary => CoverageStatus::Indeterminate,
    }
}

fn external_reference_anchor(
    attributes: &Map<String, Value>,
    root: &Path,
) -> Result<Option<(String, SourceAnchor)>, GraphError> {
    let Some(target) = optional_source_path(attributes, "source_file") else {
        return Ok(None);
    };
    let Some(origin) = optional_source_path(attributes, "origin_file") else {
        return Ok(None);
    };
    let Some(value) = attributes.get("origin_source_anchor") else {
        return Ok(None);
    };
    let target = portable_path(&target, root)?;
    let origin = portable_path(&origin, root)?;
    let mut anchor = serde_json::from_value::<SourceAnchor>(value.clone())
        .map_err(|error| raw_error("origin_source_anchor", &error.to_string()))?;
    anchor.file = portable_path(&anchor.file, root)?;
    if anchor.file != origin || !anchor.is_valid() {
        return Err(raw_error(
            "origin_source_anchor",
            "external reference anchor must be a valid range in origin_file",
        ));
    }
    Ok(Some((target, anchor)))
}

fn evidence_external_reference_diagnostic(
    diagnostics: &mut Vec<GraphDiagnostic>,
    path: &str,
    anchor: Option<SourceAnchor>,
) {
    let path = path.chars().take(512).collect::<String>();
    diagnostics.push(GraphDiagnostic {
        severity: DiagnosticSeverity::Warning,
        code: "unresolved_external_reference".to_owned(),
        message: format!("referenced path is not present in the source tree: {path}"),
        related_ids: anchor
            .as_ref()
            .map(|anchor| vec![file_id(&anchor.file)])
            .unwrap_or_default(),
        anchor,
    });
}

fn inventory_diagnostic(
    path: &str,
    status: ExtractionStatus,
    reason: Option<&str>,
) -> Option<GraphDiagnostic> {
    let (severity, code) = match status {
        ExtractionStatus::Partial
            if reason.is_some_and(|reason| reason.contains("parser recovered")) =>
        {
            (DiagnosticSeverity::Warning, "parser_recovery")
        }
        ExtractionStatus::Partial => (DiagnosticSeverity::Warning, "partial_extraction"),
        ExtractionStatus::ParseFailure => (DiagnosticSeverity::Error, "extractor_failure"),
        ExtractionStatus::Unsupported => (DiagnosticSeverity::Info, "unsupported_input"),
        ExtractionStatus::Excluded => (DiagnosticSeverity::Info, "excluded_input"),
        ExtractionStatus::Generated => (DiagnosticSeverity::Info, "generated_input"),
        ExtractionStatus::Binary => (DiagnosticSeverity::Info, "binary_input"),
        ExtractionStatus::Extracted => return None,
    };
    let reason = reason.unwrap_or("no additional reason");
    Some(GraphDiagnostic {
        severity,
        code: code.to_owned(),
        message: format!("{path}: {reason}"),
        anchor: None,
        related_ids: vec![file_id(path)],
    })
}

fn coverage_producer(attributes: &Map<String, Value>) -> String {
    optional_string(attributes, "extractor").unwrap_or_else(|| {
        optional_any_string(attributes, &["language", "lang"]).map_or_else(
            || "compass.languages.unknown".to_owned(),
            |language| format!("compass.languages.{language}"),
        )
    })
}

fn coverage_file_id(
    attributes: &Map<String, Value>,
    root: &Path,
) -> Result<Option<String>, GraphError> {
    optional_source_path(attributes, "source_file")
        .map(|path| {
            portable_path(&path, root)
                .map(|path| root.join(&path).is_file().then(|| file_id(&path)))
        })
        .transpose()
        .map(Option::flatten)
}

/// Publish resolved raw facts as a validated, deterministic Compass graph v1 document.
pub fn normalize_v1(
    extraction: Extraction,
    evidence: BuildEvidence,
) -> Result<GraphDocument, GraphError> {
    normalize_v1_with_mode(extraction, evidence, PublicationMode::Strict)
        .map(|outcome| outcome.document)
}

/// Publish the largest strict-valid graph after quarantining invalid records.
pub fn normalize_v1_best_effort(
    extraction: Extraction,
    evidence: BuildEvidence,
) -> Result<PublicationOutcome, GraphError> {
    normalize_v1_with_mode(extraction, evidence, PublicationMode::BestEffort)
}

fn normalize_v1_with_mode(
    mut extraction: Extraction,
    mut evidence: BuildEvidence,
    mode: PublicationMode,
) -> Result<PublicationOutcome, GraphError> {
    let mut quarantine = QuarantineCollector::default();
    if let Some(value) = extraction.extensions.remove(TRUSTED_GRAPH_COVERAGE) {
        let mut coverage = serde_json::from_value::<Vec<CoverageRecord>>(value)
            .map_err(|error| raw_error("graph.coverage", &error.to_string()))?;
        evidence.coverage.append(&mut coverage);
        sort_dedup_serialized(&mut evidence.coverage);
    }
    if let Some(value) = extraction.extensions.remove(TRUSTED_GRAPH_DIAGNOSTICS) {
        let mut diagnostics = serde_json::from_value::<Vec<GraphDiagnostic>>(value)
            .map_err(|error| raw_error("graph.diagnostics", &error.to_string()))?;
        evidence.diagnostics.append(&mut diagnostics);
        sort_dedup_serialized(&mut evidence.diagnostics);
    }
    normalize_file_inventory(&mut evidence.files, &evidence.repository_root)?;
    let file_facts = published_file_facts(&evidence)?;
    split_sourceless_placeholders(&mut extraction, &evidence.repository_root, &file_facts)?;
    let stub_wiring_sites =
        collect_stub_wiring_sites(&extraction.edges, &evidence.repository_root, &file_facts)?;
    resolve_or_drop_generic_symbols(
        &mut extraction,
        &mut evidence.diagnostics,
        &evidence.repository_root,
        &file_facts,
        &stub_wiring_sites,
        mode,
        &mut quarantine,
    )?;

    if mode == PublicationMode::BestEffort {
        extraction.nodes.sort_by_cached_key(raw_node_sort_key);
        extraction.edges.sort_by_cached_key(raw_edge_sort_key);
    }
    let mut id_remap = HashMap::with_capacity(extraction.nodes.len());
    let mut nodes = BTreeMap::<String, NodeRecord>::new();
    for raw in extraction.nodes {
        let raw_id = raw.id.clone();
        let diagnostic_anchor =
            best_effort_raw_anchor(&raw.attributes, &evidence.repository_root, &file_facts);
        if id_remap.contains_key(&raw_id) {
            let error = raw_error(
                &raw_id,
                "duplicate raw node ID cannot be resolved deterministically",
            );
            if mode == PublicationMode::Strict {
                return Err(error);
            }
            id_remap.remove(&raw_id);
            quarantine.omit_node(&raw_id, &error.to_string(), diagnostic_anchor);
            continue;
        }
        let trusted = raw.attributes.get(TRUSTED_NODE_RECORD).cloned();
        let normalized = match trusted {
            Some(value) => serde_json::from_value::<NodeRecord>(value)
                .map_err(|error| raw_error(&raw_id, &error.to_string())),
            None => normalize_node(
                raw,
                &evidence.repository_root,
                &file_facts,
                stub_wiring_sites.get(&raw_id),
            ),
        };
        let node = match normalized {
            Ok(node) => node,
            Err(error) if mode == PublicationMode::BestEffort => {
                quarantine.omit_node(&raw_id, &error.to_string(), diagnostic_anchor);
                continue;
            }
            Err(error) => return Err(error),
        };
        let published_id = node.id.clone();
        if let Some(existing) = nodes.get_mut(&node.id) {
            let mut merged = existing.clone();
            if let Err(error) = merge_normalized_node(&mut merged, node) {
                if mode == PublicationMode::Strict {
                    return Err(error);
                }
                quarantine.identity_collision(
                    &raw_id,
                    &error.to_string(),
                    diagnostic_anchor.or_else(|| best_effort_node_anchor(existing)),
                );
                continue;
            }
            *existing = merged;
        } else {
            nodes.insert(node.id.clone(), node);
        }
        id_remap.insert(raw_id, published_id);
    }
    for node in nodes.values_mut() {
        remap_provenance_candidates(&mut node.evidence, &id_remap);
        let Some(NodeDetails::Route(details)) = node.details.as_mut() else {
            continue;
        };
        for stage in &mut details.stages {
            if let Some(target) = stage.target.as_mut()
                && let Some(published) = id_remap.get(target)
            {
                target.clone_from(published);
            }
            for candidate in &mut stage.candidates {
                if let Some(published) = id_remap.get(&candidate.node_id) {
                    candidate.node_id.clone_from(published);
                }
            }
        }
        recompute_route_resolution(details);
    }

    let mut links = BTreeMap::<String, EdgeRecord>::new();
    for (index, raw) in extraction.edges.into_iter().enumerate() {
        let raw_identity = raw_edge_identity(&raw, index);
        let diagnostic_anchor =
            best_effort_raw_anchor(&raw.attributes, &evidence.repository_root, &file_facts);
        let source = match id_remap.get(&raw.source) {
            Some(source) => source.clone(),
            None if mode == PublicationMode::BestEffort => {
                quarantine.omit_edge(
                    &raw_identity,
                    &format!("source {} does not match a retained raw node", raw.source),
                    diagnostic_anchor,
                );
                continue;
            }
            None => {
                return Err(raw_error(
                    &format!("edge[{index}]"),
                    &format!("source {} does not match a raw node", raw.source),
                ));
            }
        };
        let target = match id_remap.get(&raw.target) {
            Some(target) => target.clone(),
            None if mode == PublicationMode::BestEffort => {
                quarantine.omit_edge(
                    &raw_identity,
                    &format!("target {} does not match a retained raw node", raw.target),
                    diagnostic_anchor,
                );
                continue;
            }
            None => {
                return Err(raw_error(
                    &format!("edge[{index}]"),
                    &format!("target {} does not match a raw node", raw.target),
                ));
            }
        };
        let trusted_edge = raw.attributes.contains_key(TRUSTED_EDGE_RECORD);
        let normalized = match raw.attributes.get(TRUSTED_EDGE_RECORD).cloned() {
            Some(value) => normalize_trusted_edge(
                raw,
                value,
                &source,
                &target,
                index,
                &evidence.repository_root,
                &file_facts,
            ),
            None => normalize_edge(
                raw,
                &source,
                &target,
                index,
                &evidence.repository_root,
                &file_facts,
            ),
        };
        let mut edge = match normalized {
            Ok(edge) => edge,
            Err(error) if mode == PublicationMode::BestEffort => {
                quarantine.omit_edge(&raw_identity, &error.to_string(), diagnostic_anchor);
                continue;
            }
            Err(error) => return Err(error),
        };
        remap_provenance_candidates(&mut edge.evidence, &id_remap);
        if !trusted_edge
            && edge.kind == EdgeKind::Tests
            && let Some(source) = nodes.get_mut(&edge.source)
        {
            source.roles.push(NodeRole::Test);
            sort_dedup_serialized(&mut source.roles);
        }
        if !trusted_edge
            && edge.kind == EdgeKind::Decorates
            && nodes
                .get(&edge.target)
                .is_some_and(|node| matches!(node.kind, NodeKind::Annotation | NodeKind::Macro))
            && nodes
                .get(&edge.source)
                .is_some_and(|node| !matches!(node.kind, NodeKind::Annotation | NodeKind::Macro))
        {
            std::mem::swap(&mut edge.source, &mut edge.target);
            let id = edge_id(
                &edge.source,
                edge.kind,
                &edge.target,
                edge.relationship_site.as_ref(),
                edge.occurrence_rule.as_ref().map(OccurrenceRule::as_str),
            );
            edge.id.clone_from(&id);
            edge.key = id;
        }
        let source_kind = nodes
            .get(&edge.source)
            .map(|node| node.kind)
            .unwrap_or(NodeKind::Variable);
        let target_kind = nodes
            .get(&edge.target)
            .map(|node| node.kind)
            .unwrap_or(NodeKind::Variable);
        let target_is_constructible = nodes.get(&edge.target).is_some_and(|node| {
            node.kind.is_constructible()
                || (node.kind == NodeKind::EnumMember && node.language.as_deref() == Some("rust"))
        });
        let unresolved_wiring_site = [&edge.source, &edge.target]
            .into_iter()
            .filter_map(|id| nodes.get(id))
            .filter(|node| is_unresolved_external_node(node))
            .find_map(|node| {
                node.evidence
                    .iter()
                    .find_map(|evidence| evidence.wiring_site.clone())
            });
        if let Some(wiring_site) = unresolved_wiring_site {
            edge.deferred = true;
            if !edge.evidence.iter().any(|evidence| {
                evidence.extractor == "compass.graph.external-placeholder"
                    && evidence.rule.as_deref() == Some("external-symbol-placeholder")
            }) {
                edge.evidence.push(Provenance {
                    origin: EvidenceOrigin::Heuristic,
                    extractor: "compass.graph.external-placeholder".to_owned(),
                    confidence: EvidenceConfidence::Inferred,
                    rule: Some("external-symbol-placeholder".to_owned()),
                    anchors: Vec::new(),
                    wiring_site: Some(edge.relationship_site.clone().unwrap_or(wiring_site)),
                    score: None,
                    candidates: Vec::new(),
                });
                sort_dedup_serialized(&mut edge.evidence);
            }
        }
        if !trusted_edge && edge.kind == EdgeKind::TypeOf && source_kind.is_callable() {
            edge.kind = EdgeKind::Returns;
            edge.details = None;
            let id = edge_id(
                &edge.source,
                edge.kind,
                &edge.target,
                edge.relationship_site.as_ref(),
                edge.occurrence_rule.as_ref().map(OccurrenceRule::as_str),
            );
            edge.id.clone_from(&id);
            edge.key = id;
        }
        if !trusted_edge && edge.kind == EdgeKind::Calls && target_is_constructible {
            edge.kind = EdgeKind::Instantiates;
            edge.details = None;
            let id = edge_id(
                &edge.source,
                edge.kind,
                &edge.target,
                edge.relationship_site.as_ref(),
                edge.occurrence_rule.as_ref().map(OccurrenceRule::as_str),
            );
            edge.id.clone_from(&id);
            edge.key = id;
            evidence.diagnostics.push(GraphDiagnostic {
                severity: DiagnosticSeverity::Info,
                code: "normalized_constructor_call".to_owned(),
                message: format!(
                    "normalized calls endpoints {} -> {} to instantiates",
                    source_kind.as_str(),
                    target_kind.as_str()
                ),
                anchor: edge.relationship_site.clone(),
                related_ids: vec![edge.source.clone(), edge.target.clone()],
            });
        }
        if edge.source == edge.target && edge.kind != EdgeKind::Calls {
            if mode == PublicationMode::BestEffort {
                quarantine.omit_edge(
                    &edge.id,
                    &format!("unsupported {} self-loop", edge.kind.as_str()),
                    edge.relationship_site.clone(),
                );
            }
            evidence.diagnostics.push(GraphDiagnostic {
                severity: DiagnosticSeverity::Warning,
                code: "dropped_non_recursive_self_loop".to_owned(),
                message: format!(
                    "dropped impossible {} self-loop on {}",
                    edge.kind.as_str(),
                    edge.source
                ),
                anchor: edge.relationship_site,
                related_ids: vec![edge.source],
            });
            continue;
        }
        if matches!(edge.kind, EdgeKind::Extends | EdgeKind::Implements) {
            let valid = source_kind.is_type()
                && match edge.kind {
                    EdgeKind::Extends => target_kind.is_type(),
                    EdgeKind::Implements => matches!(
                        target_kind,
                        NodeKind::Interface | NodeKind::Trait | NodeKind::Protocol
                    ),
                    _ => false,
                };
            if !valid {
                if mode == PublicationMode::BestEffort {
                    quarantine.omit_edge(
                        &edge.id,
                        &format!(
                            "invalid {} endpoints {} -> {}",
                            edge.kind.as_str(),
                            source_kind.as_str(),
                            target_kind.as_str()
                        ),
                        edge.relationship_site.clone(),
                    );
                }
                evidence.diagnostics.push(GraphDiagnostic {
                    severity: DiagnosticSeverity::Warning,
                    code: "dropped_invalid_inheritance_target".to_owned(),
                    message: format!(
                        "dropped invalid {} endpoints {} -> {}",
                        edge.kind.as_str(),
                        source_kind.as_str(),
                        target_kind.as_str()
                    ),
                    anchor: edge.relationship_site,
                    related_ids: vec![edge.source, edge.target],
                });
                continue;
            }
        }
        if let Some(existing) = links.get_mut(&edge.id) {
            let mut merged = existing.clone();
            if let Err(error) = merge_normalized_edge(&mut merged, edge) {
                if mode == PublicationMode::Strict {
                    return Err(error);
                }
                quarantine.omit_edge(
                    &existing.id,
                    &error.to_string(),
                    existing.relationship_site.clone(),
                );
                continue;
            }
            *existing = merged;
        } else {
            links.insert(edge.id.clone(), edge);
        }
    }
    for edge in links.values() {
        let Some(EdgeDetails::Route(edge_details)) = edge.details.as_ref() else {
            continue;
        };
        let Some(position) = edge_details.position else {
            continue;
        };
        let Some(node) = nodes.get_mut(&edge.source) else {
            continue;
        };
        let Some(NodeDetails::Route(route_details)) = node.details.as_mut() else {
            continue;
        };
        let mut became_ambiguous = false;
        if let Some(stage) = route_details
            .stages
            .iter_mut()
            .find(|stage| stage.position == position && stage.stage == edge_details.stage)
        {
            let conflicting_target = stage
                .target
                .as_ref()
                .is_some_and(|target| target != &edge.target);
            if conflicting_target || stage.resolution == ResolutionState::Ambiguous {
                let mut targets = stage
                    .candidates
                    .iter()
                    .map(|candidate| candidate.node_id.clone())
                    .collect::<BTreeSet<_>>();
                if let Some(target) = stage.target.take() {
                    targets.insert(target);
                }
                targets.insert(edge.target.clone());
                stage.candidates = targets
                    .into_iter()
                    .map(|node_id| ResolutionCandidate {
                        node_id,
                        reason: "multiple route targets remain after semantic ID remapping"
                            .to_owned(),
                        confidence: EvidenceConfidence::Ambiguous,
                        score: None,
                        anchor: None,
                    })
                    .collect();
                stage.resolution = ResolutionState::Ambiguous;
                became_ambiguous = true;
            } else {
                stage.target = Some(edge.target.clone());
                if stage.resolution == ResolutionState::Exact
                    && let [candidate] = stage.candidates.as_mut_slice()
                {
                    candidate.node_id.clone_from(&edge.target);
                }
            }
        }
        if became_ambiguous {
            route_details.resolution = ResolutionState::Ambiguous;
        }
        recompute_route_resolution(route_details);
    }

    let mut nodes = nodes.into_values().collect::<Vec<_>>();
    let mut links = links.into_values().collect::<Vec<_>>();
    for node in &mut nodes {
        for diagnostic in &mut node.diagnostics {
            remap_diagnostic_ids(diagnostic, &id_remap);
        }
    }
    for file in &mut evidence.files {
        for diagnostic in &mut file.diagnostics {
            remap_diagnostic_ids(diagnostic, &id_remap);
        }
    }
    for diagnostic in &mut evidence.diagnostics {
        remap_diagnostic_ids(diagnostic, &id_remap);
    }
    for coverage in &mut evidence.coverage {
        if let Some(file_id) = coverage.file_id.as_mut()
            && let Some(published) = id_remap.get(file_id)
        {
            file_id.clone_from(published);
        }
    }
    nodes.sort_by(|left, right| left.id.cmp(&right.id));
    links.sort_by(|left, right| {
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
    evidence
        .files
        .sort_by(|left, right| left.path.cmp(&right.path));
    sort_dedup_serialized(&mut evidence.coverage);
    evidence
        .coverage
        .sort_by(|left, right| coverage_key(left).cmp(&coverage_key(right)));
    sort_dedup_serialized(&mut evidence.diagnostics);
    evidence.diagnostics.sort_by(|left, right| {
        (left.code.as_str(), left.message.as_str())
            .cmp(&(right.code.as_str(), right.message.as_str()))
    });

    let mut document = GraphDocument {
        directed: true,
        multigraph: true,
        graph: GraphMetadata {
            schema: compass_model::code_graph::CODE_GRAPH_SCHEMA_V1.to_owned(),
            build: evidence.build,
            files: evidence.files,
            coverage: evidence.coverage,
            diagnostics: evidence.diagnostics,
        },
        nodes,
        links,
    };
    if mode == PublicationMode::BestEffort {
        sanitize_document(&mut document, &mut quarantine)?;
    }
    let omissions = quarantine.finish(&mut document.graph.diagnostics);
    sort_dedup_serialized(&mut document.graph.diagnostics);
    document.graph.diagnostics.sort_by(|left, right| {
        (left.code.as_str(), left.message.as_str())
            .cmp(&(right.code.as_str(), right.message.as_str()))
    });
    validate_code_graph(&document)?;
    Ok(PublicationOutcome {
        document,
        omissions,
    })
}

fn sanitize_document(
    document: &mut GraphDocument,
    quarantine: &mut QuarantineCollector,
) -> Result<(), GraphError> {
    let report = validate_code_graph_records(document);
    if !report.document_errors.is_empty() {
        return Err(CodeGraphValidationError {
            errors: report.document_errors,
        }
        .into());
    }

    let invalid_nodes = report
        .node_errors
        .iter()
        .map(|record| record.id.as_str())
        .collect::<HashSet<_>>();
    for record in &report.node_errors {
        quarantine.omit_node(&record.id, &record.errors.join("; "), None);
    }

    let incident_edges = document
        .links
        .iter()
        .filter(|edge| {
            invalid_nodes.contains(edge.source.as_str())
                || invalid_nodes.contains(edge.target.as_str())
        })
        .map(|edge| edge.id.clone())
        .collect::<BTreeSet<_>>();
    for edge in document
        .links
        .iter()
        .filter(|edge| incident_edges.contains(&edge.id))
    {
        quarantine.omit_edge(
            &edge.id,
            "an endpoint node was quarantined",
            edge.relationship_site.clone(),
        );
    }

    let invalid_edges = report
        .edge_errors
        .iter()
        .filter(|record| !incident_edges.contains(&record.id))
        .map(|record| record.id.clone())
        .collect::<BTreeSet<_>>();
    for record in report
        .edge_errors
        .iter()
        .filter(|record| invalid_edges.contains(&record.id))
    {
        let anchor = document
            .links
            .iter()
            .find(|edge| edge.id == record.id)
            .and_then(|edge| edge.relationship_site.clone());
        quarantine.omit_edge(&record.id, &record.errors.join("; "), anchor);
    }

    document
        .nodes
        .retain(|node| !invalid_nodes.contains(node.id.as_str()));
    document
        .links
        .retain(|edge| !incident_edges.contains(&edge.id) && !invalid_edges.contains(&edge.id));
    repair_route_topology(document);
    Ok(())
}

fn repair_route_topology(document: &mut GraphDocument) {
    let retained_nodes = document
        .nodes
        .iter()
        .map(|node| node.id.clone())
        .collect::<HashSet<_>>();
    let retained_stages = document
        .links
        .iter()
        .filter_map(|edge| {
            let EdgeDetails::Route(details) = edge.details.as_ref()? else {
                return None;
            };
            Some((
                edge.source.clone(),
                details.stage,
                details.position,
                edge.target.clone(),
            ))
        })
        .collect::<HashSet<_>>();

    for node in &mut document.nodes {
        for evidence in &mut node.evidence {
            evidence
                .candidates
                .retain(|candidate| retained_nodes.contains(candidate.node_id.as_str()));
        }
        let Some(NodeDetails::Route(details)) = node.details.as_mut() else {
            continue;
        };
        for stage in &mut details.stages {
            stage
                .candidates
                .retain(|candidate| retained_nodes.contains(candidate.node_id.as_str()));
            let retained = stage.target.as_ref().is_some_and(|target| {
                retained_stages.contains(&(
                    node.id.clone(),
                    stage.stage,
                    Some(stage.position),
                    target.clone(),
                ))
            });
            if !retained {
                stage.target = None;
                stage.resolution = if stage.candidates.len() > 1 {
                    ResolutionState::Ambiguous
                } else {
                    ResolutionState::Unresolved
                };
            }
        }
        recompute_route_resolution(details);
    }
    for edge in &mut document.links {
        for evidence in &mut edge.evidence {
            evidence
                .candidates
                .retain(|candidate| retained_nodes.contains(candidate.node_id.as_str()));
        }
    }
}

fn raw_node_sort_key(raw: &RawNodeRecord) -> String {
    let trusted = raw
        .attributes
        .get(TRUSTED_NODE_RECORD)
        .and_then(|value| serde_json::from_value::<NodeRecord>(value.clone()).ok());
    let rank = trusted.as_ref().map_or_else(
        || {
            let has_source = optional_source_path(&raw.attributes, "source_file").is_some();
            let exact = optional_string(&raw.attributes, "confidence")
                .is_some_and(|confidence| matches!(confidence.as_str(), "EXACT" | "EXTRACTED"));
            let ast = optional_string(&raw.attributes, "origin")
                .is_some_and(|origin| origin.eq_ignore_ascii_case("ast"));
            match (ast, exact, has_source) {
                (true, _, true) => 0,
                (_, true, true) => 1,
                (_, _, true) => 2,
                _ => 4,
            }
        },
        |node| {
            let exact_ast = node.evidence.iter().any(|evidence| {
                evidence.origin == EvidenceOrigin::Ast
                    && evidence.confidence == EvidenceConfidence::Exact
            });
            let exact_source = node.source.is_some()
                && node
                    .evidence
                    .iter()
                    .any(|evidence| evidence.confidence == EvidenceConfidence::Exact);
            let inferred_source = node.source.is_some();
            let exact_wiring = node.evidence.iter().any(|evidence| {
                evidence.wiring_site.is_some() && evidence.confidence == EvidenceConfidence::Exact
            });
            if exact_ast {
                0
            } else if exact_source {
                1
            } else if inferred_source {
                2
            } else if exact_wiring {
                3
            } else {
                4
            }
        },
    );
    format!(
        "{rank}\u{1f}{}\u{1f}{}",
        raw.id,
        serialized(&raw.attributes)
    )
}

fn raw_edge_sort_key(raw: &RawEdgeRecord) -> String {
    format!(
        "{}\u{1f}{}\u{1f}{}\u{1f}{}",
        raw.source,
        optional_string(&raw.attributes, "relation").unwrap_or_default(),
        raw.target,
        serialized(&raw.attributes)
    )
}

fn raw_edge_identity(raw: &RawEdgeRecord, index: usize) -> String {
    let relation =
        optional_string(&raw.attributes, "relation").unwrap_or_else(|| "<missing>".to_owned());
    format!(
        "edge[{index}] {} -[{relation}]-> {}",
        raw.source, raw.target
    )
}

fn best_effort_raw_anchor(
    attributes: &Map<String, Value>,
    root: &Path,
    file_facts: &HashMap<String, PublishedFileFacts>,
) -> Option<SourceAnchor> {
    raw_anchor(attributes, root, file_facts).ok().flatten()
}

fn best_effort_node_anchor(node: &NodeRecord) -> Option<SourceAnchor> {
    node.source.clone().or_else(|| {
        node.evidence.iter().find_map(|evidence| {
            evidence
                .anchors
                .first()
                .cloned()
                .or_else(|| evidence.wiring_site.clone())
        })
    })
}

fn remap_diagnostic_ids(diagnostic: &mut GraphDiagnostic, id_remap: &HashMap<String, String>) {
    for related_id in &mut diagnostic.related_ids {
        if let Some(published) = id_remap.get(related_id) {
            related_id.clone_from(published);
        }
    }
    diagnostic.related_ids.sort();
    diagnostic.related_ids.dedup();
}

fn normalize_trusted_edge(
    raw: RawEdgeRecord,
    trusted: Value,
    source: &str,
    target: &str,
    index: usize,
    root: &Path,
    file_facts: &HashMap<String, PublishedFileFacts>,
) -> Result<EdgeRecord, GraphError> {
    let position = format!("edge[{index}]");
    let mut edge = serde_json::from_value::<EdgeRecord>(trusted)
        .map_err(|error| raw_error(&position, &error.to_string()))?;
    let embedded_source = edge.source.clone();
    let embedded_target = edge.target.clone();
    let embedded_has_ast = edge
        .evidence
        .iter()
        .any(|evidence| evidence.origin == EvidenceOrigin::Ast);
    if edge
        .occurrence_rule
        .as_ref()
        .is_some_and(OccurrenceRule::is_endpoint_rewrite)
    {
        return Err(raw_error(
            &position,
            "trusted occurrence rule uses a reserved endpoint rewrite name",
        ));
    }
    for evidence in &edge.evidence {
        evidence
            .validate_endpoint_rewrite()
            .map_err(|error| raw_error(&position, &error.to_string()))?;
    }
    if let Some(raw_rule) = raw.attributes.get(OCCURRENCE_RULE_ATTRIBUTE) {
        let raw_rule = raw_rule
            .as_str()
            .and_then(|rule| OccurrenceRule::new(rule.to_owned()))
            .ok_or_else(|| {
                raw_error(&position, "raw occurrence rule must be a non-empty string")
            })?;
        if edge.occurrence_rule.as_ref() != Some(&raw_rule) {
            return Err(raw_error(
                &position,
                &format!(
                    "conflicting raw occurrence rule {:?} does not match trusted typed identity {:?}",
                    raw_rule.as_str(),
                    edge.occurrence_rule.as_ref().map(OccurrenceRule::as_str),
                ),
            ));
        }
    }

    let owner = format!("{position} {source} -[{}]-> {target}", edge.kind.as_str());
    let mut added_evidence = Vec::new();
    let evidence_context = RawEdgeEvidenceContext {
        owner: &owner,
        root,
        file_facts,
        normalization_rule: None,
        heuristic_default: false,
    };
    append_raw_endpoint_rewrite_evidence(
        &mut added_evidence,
        &raw.attributes,
        edge.relationship_site.clone(),
        &evidence_context,
    )?;
    let consume_incremental_endpoint_remap = match raw
        .attributes
        .get(CONSUME_INCREMENTAL_ENDPOINT_REMAP_ATTRIBUTE)
    {
        None => false,
        Some(Value::Bool(true)) => true,
        Some(_) => {
            return Err(raw_error(
                &position,
                "incremental endpoint remap consumption marker must be true",
            ));
        }
    };
    if consume_incremental_endpoint_remap
        && (embedded_has_ast
            || edge.relationship_site.is_some()
            || !added_evidence.iter().any(|evidence| {
                evidence.rule.as_deref()
                    == Some(EndpointRewriteRule::IncrementalAstEndpointRemap.as_str())
            }))
    {
        return Err(raw_error(
            &position,
            "semantic residue requires current incremental endpoint remap evidence",
        ));
    }
    if let Some(constituents) = raw
        .attributes
        .get(COALESCED_EDGE_EVIDENCE)
        .and_then(Value::as_array)
    {
        for constituent in constituents {
            let Some(attributes) = constituent.as_object() else {
                continue;
            };
            let constituent_site = raw_anchor(attributes, root, file_facts)?;
            append_raw_edge_evidence(
                &mut added_evidence,
                attributes,
                constituent_site,
                &evidence_context,
            )?;
        }
    }
    if (embedded_source != source || embedded_target != target)
        && !added_evidence.iter().any(|evidence| {
            evidence
                .rule
                .as_deref()
                .and_then(EndpointRewriteRule::from_wire_name)
                .is_some()
        })
    {
        return Err(raw_error(
            &position,
            "changed trusted endpoints require current endpoint rewrite evidence",
        ));
    }
    if !embedded_has_ast
        && added_evidence
            .iter()
            .any(|evidence| evidence.origin == EvidenceOrigin::Ast)
    {
        added_evidence.retain(|evidence| {
            evidence.rule.as_deref()
                != Some(EndpointRewriteRule::IncrementalAstEndpointRemap.as_str())
        });
    }
    if consume_incremental_endpoint_remap {
        // A current incremental remap may transport a site-less semantic assertion to its
        // canonical endpoints, but it is not producer evidence for that assertion. Validation
        // above consumes the exact rewrite proof before the transient fact is published.
        added_evidence.retain(|evidence| {
            evidence.rule.as_deref()
                != Some(EndpointRewriteRule::IncrementalAstEndpointRemap.as_str())
        });
    }
    edge.evidence.append(&mut added_evidence);
    sort_dedup_serialized(&mut edge.evidence);
    edge.source = source.to_owned();
    edge.target = target.to_owned();
    edge.id = edge_id(
        source,
        edge.kind,
        target,
        edge.relationship_site.as_ref(),
        edge.occurrence_rule.as_ref().map(OccurrenceRule::as_str),
    );
    edge.key.clone_from(&edge.id);
    Ok(edge)
}

fn is_unresolved_external_node(node: &NodeRecord) -> bool {
    node.source.is_none()
        && node.evidence.iter().any(|evidence| {
            evidence.origin == EvidenceOrigin::Heuristic
                && evidence.confidence == EvidenceConfidence::Inferred
                && evidence.rule.as_deref() == Some("external-symbol-placeholder")
                && evidence.wiring_site.is_some()
        })
}

fn collect_stub_wiring_sites(
    edges: &[RawEdgeRecord],
    root: &Path,
    file_facts: &HashMap<String, PublishedFileFacts>,
) -> Result<HashMap<String, SourceAnchor>, GraphError> {
    let mut sites = HashMap::new();
    for edge in edges {
        let Some(anchor) = raw_anchor(&edge.attributes, root, file_facts)? else {
            continue;
        };
        for endpoint in [&edge.source, &edge.target] {
            sites
                .entry(endpoint.clone())
                .and_modify(|existing: &mut SourceAnchor| {
                    if anchor_key(&anchor) < anchor_key(existing) {
                        existing.clone_from(&anchor);
                    }
                })
                .or_insert_with(|| anchor.clone());
        }
    }
    Ok(sites)
}

fn split_sourceless_placeholders(
    extraction: &mut Extraction,
    root: &Path,
    file_facts: &HashMap<String, PublishedFileFacts>,
) -> Result<(), GraphError> {
    let sourceless = extraction
        .nodes
        .iter()
        .filter(|node| {
            node.attributes.get(TRUSTED_NODE_RECORD).is_none()
                && optional_source_path(&node.attributes, "source_file").is_none()
                && !node.attributes.contains_key("source_anchor")
        })
        .map(|node| node.id.clone())
        .collect::<BTreeSet<_>>();
    if sourceless.is_empty() {
        return Ok(());
    }
    let node_attributes = extraction
        .nodes
        .iter()
        .map(|node| (node.id.clone(), node.attributes.clone()))
        .collect::<HashMap<_, _>>();
    let mut scopes = HashMap::<String, BTreeSet<String>>::new();
    let mut inferred_kinds = HashMap::<(String, String), &'static str>::new();
    for edge in &extraction.edges {
        let Some(anchor) = raw_anchor(&edge.attributes, root, file_facts)? else {
            continue;
        };
        for (endpoint, counterpart) in [(&edge.source, &edge.target), (&edge.target, &edge.source)]
        {
            if !sourceless.contains(endpoint) {
                continue;
            }
            let counterpart_attributes = node_attributes.get(counterpart.as_str());
            scopes
                .entry(endpoint.clone())
                .or_default()
                .insert(placeholder_scope_key(
                    &edge.attributes,
                    counterpart_attributes,
                    &anchor,
                ));
        }
        if sourceless.contains(&edge.target)
            && let Some(kind) = inferred_external_target_kind(&edge.attributes)
            && let Some(attributes) = node_attributes.get(&edge.target)
        {
            let scope =
                placeholder_scope_key(&edge.attributes, node_attributes.get(&edge.source), &anchor);
            let qualified = optional_any_string(
                attributes,
                &["qualified_name", "qualifiedName", "name", "label"],
            )
            .unwrap_or_else(|| edge.target.clone());
            inferred_kinds
                .entry((scope, qualified))
                .and_modify(|existing| {
                    if kind == "interface" {
                        *existing = kind;
                    }
                })
                .or_insert(kind);
        }
    }

    let mut split_ids = HashMap::<String, BTreeMap<String, String>>::new();
    let mut expanded = Vec::with_capacity(extraction.nodes.len() + scopes.len());
    for node in std::mem::take(&mut extraction.nodes) {
        if !sourceless.contains(&node.id) {
            expanded.push(node);
            continue;
        }
        let Some(node_scopes) = scopes.get(&node.id).filter(|scopes| !scopes.is_empty()) else {
            let mut node = node;
            mark_external_placeholder(&mut node.attributes, None, None);
            expanded.push(node);
            continue;
        };
        let qualified = optional_any_string(
            &node.attributes,
            &["qualified_name", "qualifiedName", "name", "label"],
        )
        .unwrap_or_else(|| node.id.clone());
        let clones = node_scopes
            .iter()
            .map(|scope| {
                let clone_id = if node_scopes.len() == 1 {
                    node.id.clone()
                } else {
                    let digest = Sha256::digest(scope.as_bytes());
                    format!("{}#scope-{digest:x}", node.id)
                };
                (scope.clone(), clone_id)
            })
            .collect::<BTreeMap<_, _>>();
        for (scope, clone_id) in &clones {
            let mut clone = node.clone();
            clone.id.clone_from(clone_id);
            let inferred_kind = inferred_kinds
                .get(&(scope.clone(), qualified.clone()))
                .copied();
            mark_external_placeholder(&mut clone.attributes, Some(scope), inferred_kind);
            expanded.push(clone);
        }
        split_ids.insert(node.id, clones);
    }
    extraction.nodes = expanded;

    for edge in &mut extraction.edges {
        let Some(anchor) = raw_anchor(&edge.attributes, root, file_facts)? else {
            continue;
        };
        let original_source = edge.source.clone();
        let original_target = edge.target.clone();
        if let Some(clones) = split_ids.get(&original_source) {
            let counterpart_attributes = node_attributes.get(&original_target);
            let scope = placeholder_scope_key(&edge.attributes, counterpart_attributes, &anchor);
            if let Some(clone_id) = clones.get(&scope) {
                edge.source.clone_from(clone_id);
            }
        }
        if let Some(clones) = split_ids.get(&original_target) {
            let counterpart_attributes = node_attributes.get(&original_source);
            let scope = placeholder_scope_key(&edge.attributes, counterpart_attributes, &anchor);
            if let Some(clone_id) = clones.get(&scope) {
                edge.target.clone_from(clone_id);
            }
        }
    }
    Ok(())
}

fn placeholder_scope_key(
    edge: &Map<String, Value>,
    counterpart: Option<&Map<String, Value>>,
    anchor: &SourceAnchor,
) -> String {
    let scoped = |keys: &[&str]| {
        optional_any_string(edge, keys)
            .or_else(|| counterpart.and_then(|attributes| optional_any_string(attributes, keys)))
    };
    [
        scoped(&["language", "lang"]).unwrap_or_default(),
        scoped(&["package", "package_name"]).unwrap_or_default(),
        scoped(&["module", "namespace"]).unwrap_or_default(),
        anchor.file.clone(),
    ]
    .join("\u{1f}")
}

fn inferred_external_target_kind(attributes: &Map<String, Value>) -> Option<&'static str> {
    match optional_string(attributes, "relation").as_deref() {
        Some("implements" | "scip_impl" | "mixes_in") => Some("interface"),
        Some("extends" | "inherits") => Some("class"),
        Some("calls" | "indirect_call") => Some("function"),
        Some("type_of" | "returns" | "scip_typed") => Some("type_alias"),
        Some("references")
            if matches!(
                optional_string(attributes, "context").as_deref(),
                Some(
                    "field"
                        | "generic_arg"
                        | "parameter_type"
                        | "return_type"
                        | "type"
                        | "type_annotation"
                )
            ) =>
        {
            Some("type_alias")
        }
        _ => None,
    }
}

fn resolve_or_drop_generic_symbols(
    extraction: &mut Extraction,
    diagnostics: &mut Vec<GraphDiagnostic>,
    root: &Path,
    file_facts: &HashMap<String, PublishedFileFacts>,
    wiring_sites: &HashMap<String, SourceAnchor>,
    mode: PublicationMode,
    quarantine: &mut QuarantineCollector,
) -> Result<(), GraphError> {
    let generic_ids = extraction
        .nodes
        .iter()
        .filter(|node| {
            node.attributes.get(TRUSTED_NODE_RECORD).is_none()
                && node
                    .attributes
                    .get("file_type")
                    .and_then(Value::as_str)
                    .unwrap_or("code")
                    == "code"
                && node
                    .attributes
                    .get("symbol_kind")
                    .or_else(|| node.attributes.get("type"))
                    .and_then(Value::as_str)
                    .is_none_or(|kind| kind == "symbol")
        })
        .map(|node| node.id.clone())
        .collect::<BTreeSet<_>>();
    if generic_ids.is_empty() {
        return Ok(());
    }

    let mut inferred = HashMap::<String, BTreeSet<&'static str>>::new();
    for edge in &extraction.edges {
        if generic_ids.contains(&edge.target)
            && let Some(kind) = inferred_external_target_kind(&edge.attributes)
        {
            inferred
                .entry(edge.target.clone())
                .or_default()
                .insert(kind);
        }
    }

    for node in &mut extraction.nodes {
        let Some(kinds) = inferred.get(&node.id).filter(|kinds| kinds.len() == 1) else {
            continue;
        };
        if let Some(kind) = kinds.first() {
            node.attributes
                .insert("symbol_kind".to_owned(), Value::String((*kind).to_owned()));
        }
    }

    let dropped = extraction
        .nodes
        .iter()
        .filter(|node| {
            generic_ids.contains(&node.id)
                && node
                    .attributes
                    .get("symbol_kind")
                    .or_else(|| node.attributes.get("type"))
                    .and_then(Value::as_str)
                    .is_none_or(|kind| kind == "symbol")
        })
        .map(|node| node.id.clone())
        .collect::<BTreeSet<_>>();
    if dropped.is_empty() {
        return Ok(());
    }

    for node in extraction
        .nodes
        .iter()
        .filter(|node| dropped.contains(&node.id))
    {
        let incident_count = extraction
            .edges
            .iter()
            .filter(|edge| edge.source == node.id || edge.target == node.id)
            .count();
        let anchor = raw_anchor(&node.attributes, root, file_facts)?
            .or(raw_origin_anchor(&node.attributes, root, file_facts)?)
            .or_else(|| wiring_sites.get(&node.id).cloned());
        if anchor.is_none()
            && optional_string(&node.attributes, "extractor").as_deref()
                == Some("compass.graph.external-placeholder")
            && mode == PublicationMode::Strict
        {
            return Err(raw_error(
                &node.id,
                "unresolved external placeholder requires an exact wiring site",
            ));
        }
        if mode == PublicationMode::BestEffort {
            quarantine.omit_node(
                &node.id,
                "no exact node kind or wiring site could be inferred",
                anchor.clone(),
            );
        }
        diagnostics.push(GraphDiagnostic {
            severity: DiagnosticSeverity::Warning,
            code: "unresolved_node_kind".to_owned(),
            message: format!(
                "omitted generic symbol {} and {incident_count} incident relationships because no exact node kind could be inferred",
                node.id
            ),
            anchor,
            related_ids: Vec::new(),
        });
    }
    if mode == PublicationMode::BestEffort {
        for edge in extraction
            .edges
            .iter()
            .filter(|edge| dropped.contains(&edge.source) || dropped.contains(&edge.target))
        {
            quarantine.omit_edge(
                &raw_edge_identity(edge, 0),
                "an endpoint node was quarantined",
                best_effort_raw_anchor(&edge.attributes, root, file_facts),
            );
        }
    }
    extraction.nodes.retain(|node| !dropped.contains(&node.id));
    extraction
        .edges
        .retain(|edge| !dropped.contains(&edge.source) && !dropped.contains(&edge.target));
    Ok(())
}

fn mark_external_placeholder(
    attributes: &mut Map<String, Value>,
    stable_scope: Option<&str>,
    inferred_kind: Option<&str>,
) {
    attributes.insert(
        "extractor".to_owned(),
        Value::String("compass.graph.external-placeholder".to_owned()),
    );
    if let Some(scope) = stable_scope {
        attributes.insert(
            "external_identity_scope".to_owned(),
            Value::String(scope.to_owned()),
        );
        let mut parts = scope.split('\u{1f}');
        let language = parts.next().unwrap_or_default();
        let package = parts.next().unwrap_or_default();
        let module = parts.next().unwrap_or_default();
        for key in [
            "language",
            "lang",
            "package",
            "package_name",
            "module",
            "namespace",
            "lexical_owner",
            "declaring_scope",
            "origin_file",
        ] {
            attributes.remove(key);
        }
        if !language.is_empty() {
            attributes.insert("language".to_owned(), Value::String(language.to_owned()));
        }
        if !package.is_empty() {
            attributes.insert("package".to_owned(), Value::String(package.to_owned()));
        }
        if !module.is_empty() {
            attributes.insert("module".to_owned(), Value::String(module.to_owned()));
        }
    }
    if attributes
        .get("symbol_kind")
        .or_else(|| attributes.get("type"))
        .and_then(Value::as_str)
        .is_none_or(|kind| kind == "symbol" || kind == "variable")
        && let Some(kind) = inferred_kind
    {
        attributes.insert("symbol_kind".to_owned(), Value::String(kind.to_owned()));
    }
}

fn anchor_key(anchor: &SourceAnchor) -> (&str, u64, u64) {
    (&anchor.file, anchor.start_byte, anchor.end_byte)
}

fn recompute_route_resolution(details: &mut RouteNodeDetails) {
    for stage in &mut details.stages {
        sort_dedup_candidates(&mut stage.candidates);
        let mut targets = stage
            .candidates
            .iter()
            .map(|candidate| candidate.node_id.clone())
            .collect::<BTreeSet<_>>();
        if let Some(target) = &stage.target {
            targets.insert(target.clone());
        }
        match targets.len() {
            0 => {
                stage.resolution = ResolutionState::Unresolved;
                stage.target = None;
            }
            1 => {
                let exact = stage.target.is_some()
                    || stage.candidates.iter().any(|candidate| {
                        candidate.confidence == EvidenceConfidence::Exact
                            && Some(&candidate.node_id) == targets.iter().next()
                    });
                if exact {
                    stage.resolution = ResolutionState::Exact;
                    stage.target = targets.into_iter().next();
                } else {
                    stage.resolution = ResolutionState::Unresolved;
                    stage.target = None;
                }
            }
            _ => {
                stage.resolution = ResolutionState::Ambiguous;
                stage.target = None;
            }
        }
    }
    details.resolution = if details
        .stages
        .iter()
        .any(|stage| stage.resolution == ResolutionState::Ambiguous)
    {
        ResolutionState::Ambiguous
    } else if details
        .stages
        .iter()
        .any(|stage| stage.resolution == ResolutionState::Unresolved)
    {
        ResolutionState::Unresolved
    } else {
        ResolutionState::Exact
    };
}

const fn route_candidate_confidence_rank(confidence: EvidenceConfidence) -> u8 {
    match confidence {
        EvidenceConfidence::Exact => 0,
        EvidenceConfidence::Inferred => 1,
        EvidenceConfidence::Ambiguous => 2,
    }
}

fn merge_normalized_node(
    existing: &mut NodeRecord,
    mut duplicate: NodeRecord,
) -> Result<(), GraphError> {
    let existing_has_ast = existing
        .evidence
        .iter()
        .any(|evidence| evidence.origin == EvidenceOrigin::Ast);
    let duplicate_has_ast = duplicate
        .evidence
        .iter()
        .any(|evidence| evidence.origin == EvidenceOrigin::Ast);
    if duplicate_has_ast && !existing_has_ast {
        std::mem::swap(existing, &mut duplicate);
    }
    let current_ast_is_authoritative =
        existing_has_ast != duplicate_has_ast && (existing_has_ast || duplicate_has_ast);
    let compatible_route_registration = existing.kind == NodeKind::Route
        && duplicate.kind == NodeKind::Route
        && existing.name == duplicate.name
        && existing.qualified_name == duplicate.qualified_name
        && existing.language == duplicate.language
        && existing.framework == duplicate.framework;
    if !current_ast_is_authoritative
        && !compatible_route_registration
        && (existing.kind != duplicate.kind
            || existing.name != duplicate.name
            || existing.qualified_name != duplicate.qualified_name
            || existing.language != duplicate.language
            || existing.framework != duplicate.framework
            || existing.source != duplicate.source)
    {
        return Err(raw_error(
            &existing.id,
            &format!(
                "stable node identity collision has incompatible semantic records: \
                 existing=({:?}, {:?}, {:?}, {:?}, {:?}, {:?}, {}) \
                 duplicate=({:?}, {:?}, {:?}, {:?}, {:?}, {:?}, {})",
                existing.kind,
                existing.name,
                existing.qualified_name,
                existing.language,
                existing.framework,
                existing.source,
                serialized(&existing.details),
                duplicate.kind,
                duplicate.name,
                duplicate.qualified_name,
                duplicate.language,
                duplicate.framework,
                duplicate.source,
                serialized(&duplicate.details),
            ),
        ));
    }
    existing.roles.append(&mut duplicate.roles);
    sort_dedup_serialized(&mut existing.roles);
    existing.evidence.append(&mut duplicate.evidence);
    sort_dedup_serialized(&mut existing.evidence);
    existing.coverage.append(&mut duplicate.coverage);
    sort_dedup_serialized(&mut existing.coverage);
    existing.diagnostics.append(&mut duplicate.diagnostics);
    sort_dedup_serialized(&mut existing.diagnostics);
    if !current_ast_is_authoritative {
        if compatible_route_registration {
            existing.source = deterministic_option(existing.source.take(), duplicate.source.take());
        }
        existing.details = deterministic_option(existing.details.take(), duplicate.details);
    }
    existing.community = deterministic_option(existing.community.take(), duplicate.community);
    Ok(())
}

fn merge_normalized_edge(
    existing: &mut EdgeRecord,
    mut duplicate: EdgeRecord,
) -> Result<(), GraphError> {
    let existing_has_ast = existing
        .evidence
        .iter()
        .any(|evidence| evidence.origin == EvidenceOrigin::Ast);
    let duplicate_has_ast = duplicate
        .evidence
        .iter()
        .any(|evidence| evidence.origin == EvidenceOrigin::Ast);
    if duplicate_has_ast && !existing_has_ast {
        std::mem::swap(existing, &mut duplicate);
    }
    if existing.source != duplicate.source
        || existing.target != duplicate.target
        || existing.kind != duplicate.kind
        || existing.occurrence_rule != duplicate.occurrence_rule
        || existing.relationship_site != duplicate.relationship_site
    {
        return Err(raw_error(
            &existing.id,
            "stable edge identity collision has incompatible semantic records",
        ));
    }
    let current_ast_is_authoritative =
        existing_has_ast != duplicate_has_ast && (existing_has_ast || duplicate_has_ast);
    existing.evidence.append(&mut duplicate.evidence);
    if current_ast_is_authoritative {
        existing.evidence.retain(|evidence| {
            evidence.rule.as_deref()
                != Some(EndpointRewriteRule::IncrementalAstEndpointRemap.as_str())
        });
    }
    sort_dedup_serialized(&mut existing.evidence);
    existing.diagnostics.append(&mut duplicate.diagnostics);
    sort_dedup_serialized(&mut existing.diagnostics);
    if !current_ast_is_authoritative {
        existing.details = deterministic_option(existing.details.take(), duplicate.details);
        existing.weight = deterministic_option(existing.weight.take(), duplicate.weight);
        existing.context = deterministic_option(existing.context.take(), duplicate.context);
    }
    existing.deferred |= duplicate.deferred;
    Ok(())
}

fn deterministic_option<T: Serialize>(left: Option<T>, right: Option<T>) -> Option<T> {
    match (left, right) {
        (Some(left), Some(right)) => {
            if serialized(&left) <= serialized(&right) {
                Some(left)
            } else {
                Some(right)
            }
        }
        (left @ Some(_), None) => left,
        (None, right) => right,
    }
}

fn sort_dedup_serialized<T: Serialize>(values: &mut Vec<T>) {
    values.sort_by_cached_key(serialized);
    values.dedup_by(|left, right| serialized(left) == serialized(right));
}

fn serialized<T: Serialize>(value: &T) -> String {
    serde_json::to_string(value).unwrap_or_default()
}

/// Convert the canonical internal analysis graph at the sole v1 publication boundary.
pub fn normalize_document_v1(
    document: &compass_model::GraphDocument,
    repository_root: &Path,
    configuration_digest: impl Into<String>,
    source_commit: Option<&str>,
) -> Result<GraphDocument, GraphError> {
    normalize_document_v1_with_inventory(
        document,
        repository_root,
        configuration_digest,
        source_commit,
        Vec::new(),
    )
}

pub fn normalize_document_v1_with_inventory(
    document: &compass_model::GraphDocument,
    repository_root: &Path,
    configuration_digest: impl Into<String>,
    source_commit: Option<&str>,
    inventory: Vec<InventoryEvidence>,
) -> Result<GraphDocument, GraphError> {
    let (extraction, evidence) = document_publication_input(
        document,
        repository_root,
        configuration_digest,
        source_commit,
        inventory,
    )?;
    normalize_v1(extraction, evidence)
}

pub fn normalize_document_v1_with_inventory_best_effort(
    document: &compass_model::GraphDocument,
    repository_root: &Path,
    configuration_digest: impl Into<String>,
    source_commit: Option<&str>,
    inventory: Vec<InventoryEvidence>,
) -> Result<PublicationOutcome, GraphError> {
    let (extraction, evidence) = document_publication_input(
        document,
        repository_root,
        configuration_digest,
        source_commit,
        inventory,
    )?;
    normalize_v1_best_effort(extraction, evidence)
}

/// Publish an owned analysis document without cloning its node and edge facts.
///
/// Callers that already hold a publication-only document can transfer its
/// buffers across the v1 boundary and avoid retaining a second full raw graph.
pub fn normalize_document_v1_with_inventory_best_effort_owned(
    document: compass_model::GraphDocument,
    repository_root: &Path,
    configuration_digest: impl Into<String>,
    source_commit: Option<&str>,
    inventory: Vec<InventoryEvidence>,
) -> Result<PublicationOutcome, GraphError> {
    let (extraction, evidence) = document_publication_input_owned(
        document,
        repository_root,
        configuration_digest,
        source_commit,
        inventory,
    )?;
    normalize_v1_best_effort(extraction, evidence)
}

fn document_publication_input(
    document: &compass_model::GraphDocument,
    repository_root: &Path,
    configuration_digest: impl Into<String>,
    source_commit: Option<&str>,
    inventory: Vec<InventoryEvidence>,
) -> Result<(Extraction, BuildEvidence), GraphError> {
    let mut extensions = Map::new();
    if let Some(diagnostics) = document.graph.get(TRUSTED_GRAPH_DIAGNOSTICS) {
        extensions.insert(TRUSTED_GRAPH_DIAGNOSTICS.to_owned(), diagnostics.clone());
    }
    let extraction = Extraction {
        nodes: document
            .nodes
            .iter()
            .map(|node| RawNodeRecord {
                id: node.id.clone(),
                attributes: node.attributes.clone(),
            })
            .collect(),
        edges: document
            .links
            .iter()
            .map(|edge| RawEdgeRecord {
                source: edge.source.clone(),
                target: edge.target.clone(),
                attributes: edge.attributes.clone(),
            })
            .collect(),
        extensions,
        ..Extraction::default()
    };
    let mut evidence =
        BuildEvidence::from_extraction(repository_root, &extraction, configuration_digest)?;
    evidence.include_inventory(inventory)?;
    evidence.build.source_commit = source_commit.map(str::to_owned);
    Ok((extraction, evidence))
}

fn document_publication_input_owned(
    document: compass_model::GraphDocument,
    repository_root: &Path,
    configuration_digest: impl Into<String>,
    source_commit: Option<&str>,
    inventory: Vec<InventoryEvidence>,
) -> Result<(Extraction, BuildEvidence), GraphError> {
    let mut extensions = Map::new();
    if let Some(diagnostics) = document.graph.get(TRUSTED_GRAPH_DIAGNOSTICS) {
        extensions.insert(TRUSTED_GRAPH_DIAGNOSTICS.to_owned(), diagnostics.clone());
    }
    let extraction = Extraction {
        nodes: document
            .nodes
            .into_iter()
            .map(|node| RawNodeRecord {
                id: node.id,
                attributes: node.attributes,
            })
            .collect(),
        edges: document
            .links
            .into_iter()
            .map(|edge| RawEdgeRecord {
                source: edge.source,
                target: edge.target,
                attributes: edge.attributes,
            })
            .collect(),
        extensions,
        ..Extraction::default()
    };
    let mut evidence =
        BuildEvidence::from_extraction(repository_root, &extraction, configuration_digest)?;
    evidence.include_inventory(inventory)?;
    evidence.build.source_commit = source_commit.map(str::to_owned);
    Ok((extraction, evidence))
}

/// Project trusted v1 records back into flexible facts for incremental recomposition.
#[must_use]
pub fn extraction_from_v1(document: &GraphDocument) -> Extraction {
    let mut extensions = Map::new();
    extensions.insert(
        TRUSTED_GRAPH_COVERAGE.to_owned(),
        serde_json::to_value(&document.graph.coverage).unwrap_or(Value::Null),
    );
    extensions.insert(
        TRUSTED_GRAPH_DIAGNOSTICS.to_owned(),
        serde_json::to_value(&document.graph.diagnostics).unwrap_or(Value::Null),
    );
    Extraction {
        nodes: document.nodes.iter().map(raw_node_from_v1).collect(),
        edges: document.links.iter().map(raw_edge_from_v1).collect(),
        extensions,
        ..Extraction::default()
    }
}

fn raw_node_from_v1(node: &NodeRecord) -> RawNodeRecord {
    let mut attributes = node
        .properties()
        .map(|(key, value)| (key.to_owned(), value))
        .collect::<Map<_, _>>();
    attributes.insert(
        "symbol_kind".to_owned(),
        Value::String(node.kind.as_str().to_owned()),
    );
    if let Some(source) = &node.source {
        attributes.insert(
            "source_anchor".to_owned(),
            serde_json::to_value(source).unwrap_or(Value::Null),
        );
    }
    if let Some(evidence) = node.evidence.first() {
        insert_raw_evidence(&mut attributes, evidence);
    }
    if let Some(details) = &node.details {
        insert_raw_node_details(&mut attributes, details);
    }
    attributes.insert(
        TRUSTED_NODE_RECORD.to_owned(),
        serde_json::to_value(node).unwrap_or(Value::Null),
    );
    RawNodeRecord {
        id: node.id.clone(),
        attributes,
    }
}

fn raw_edge_from_v1(edge: &EdgeRecord) -> RawEdgeRecord {
    let mut attributes = edge
        .properties()
        .map(|(key, value)| (key.to_owned(), value))
        .collect::<Map<_, _>>();
    if let Some(site) = authoritative_edge_rewrite_site(edge) {
        attributes.insert(
            "source_anchor".to_owned(),
            serde_json::to_value(&site).unwrap_or(Value::Null),
        );
    }
    if let Some(rule) = &edge.occurrence_rule {
        attributes.insert(
            OCCURRENCE_RULE_ATTRIBUTE.to_owned(),
            Value::String(rule.as_str().to_owned()),
        );
    }
    if let Some(evidence) = edge.evidence.first() {
        insert_raw_evidence(&mut attributes, evidence);
    }
    if let Some(details) = &edge.details {
        insert_raw_edge_details(&mut attributes, details);
    }
    attributes.insert(
        TRUSTED_EDGE_RECORD.to_owned(),
        serde_json::to_value(edge).unwrap_or(Value::Null),
    );
    RawEdgeRecord {
        source: edge.source.clone(),
        target: edge.target.clone(),
        attributes,
    }
}

fn authoritative_edge_rewrite_site(edge: &EdgeRecord) -> Option<SourceAnchor> {
    if let Some(site) = edge
        .relationship_site
        .as_ref()
        .filter(|site| site.is_valid() && site.start_byte < site.end_byte)
    {
        return Some(site.clone());
    }
    let non_rewrite = edge.evidence.iter().filter(|evidence| {
        !evidence.rule.as_deref().is_some_and(|rule| {
            serde_json::from_value::<EndpointRewriteRule>(Value::String(rule.to_owned())).is_ok()
        })
    });
    let producer_rule = edge.occurrence_rule.as_ref().map(OccurrenceRule::as_str);
    let mut preferred = non_rewrite
        .clone()
        .filter(|evidence| producer_rule.is_some_and(|rule| evidence.rule.as_deref() == Some(rule)))
        .flat_map(provenance_exact_sites)
        .collect::<Vec<_>>();
    if preferred.is_empty() {
        preferred.extend(non_rewrite.flat_map(provenance_exact_sites));
    }
    preferred.sort_by(|left, right| anchor_key(left).cmp(&anchor_key(right)));
    preferred.into_iter().next()
}

fn provenance_exact_sites(evidence: &Provenance) -> impl Iterator<Item = SourceAnchor> + '_ {
    evidence
        .anchors
        .iter()
        .chain(evidence.wiring_site.iter())
        .filter(|site| site.is_valid() && site.start_byte < site.end_byte)
        .cloned()
}

fn insert_raw_evidence(attributes: &mut Map<String, Value>, evidence: &Provenance) {
    attributes.insert(
        "_origin".to_owned(),
        Value::String(evidence.origin.as_str().to_owned()),
    );
    attributes.insert(
        "confidence".to_owned(),
        Value::String(evidence.confidence.legacy_str().to_owned()),
    );
    attributes.insert(
        "extractor".to_owned(),
        Value::String(evidence.extractor.clone()),
    );
    if let Some(rule) = &evidence.rule {
        attributes.insert("rule".to_owned(), Value::String(rule.clone()));
    } else {
        attributes.remove("rule");
    }
    if let Some(score) = evidence.score {
        attributes.insert("confidence_score".to_owned(), Value::from(score));
    } else {
        attributes.remove("confidence_score");
    }
    if !evidence.candidates.is_empty() {
        attributes.insert(
            "candidates".to_owned(),
            serde_json::to_value(&evidence.candidates).unwrap_or(Value::Null),
        );
    }
}

fn insert_raw_node_details(attributes: &mut Map<String, Value>, details: &NodeDetails) {
    match details {
        NodeDetails::File(_) | NodeDetails::Resource(_) => {}
        NodeDetails::Symbol(details) => {
            insert_optional_string(attributes, "signature", details.signature.as_ref());
            insert_optional_string(
                attributes,
                "overload_discriminator",
                details.overload_discriminator.as_ref(),
            );
            insert_optional_string(
                attributes,
                "declaring_type",
                details.declaring_type.as_ref(),
            );
            insert_optional_string(
                attributes,
                "signature_hash",
                details.signature_digest.as_ref(),
            );
            insert_optional_string(
                attributes,
                "implementation_hash",
                details.implementation_digest.as_ref(),
            );
            insert_optional_string(attributes, "source_hash", details.source_digest.as_ref());
            if !details.modifiers.is_empty() {
                attributes.insert("modifiers".to_owned(), serde_json::json!(details.modifiers));
            }
        }
        NodeDetails::ImportExport(details) => {
            attributes.insert(
                "specifier".to_owned(),
                Value::String(details.specifier.clone()),
            );
            insert_optional_string(attributes, "imported_name", details.imported_name.as_ref());
            insert_optional_string(attributes, "local_name", details.local_name.as_ref());
            attributes.insert("type_only".to_owned(), Value::Bool(details.type_only));
        }
        NodeDetails::Route(details) => {
            attributes.insert(
                "operation".to_owned(),
                Value::String(details.operation.clone()),
            );
            attributes.insert("path".to_owned(), Value::String(details.path.clone()));
            insert_optional_string(attributes, "original_path", details.original_path.as_ref());
            attributes.insert(
                "declaring_scope".to_owned(),
                Value::String(details.declaring_scope.clone()),
            );
            attributes.insert(
                "resolution".to_owned(),
                serde_json::to_value(details.resolution).unwrap_or(Value::Null),
            );
            attributes.insert(
                "middleware_count".to_owned(),
                Value::from(details.middleware_count),
            );
            if !details.stages.is_empty() {
                attributes.insert(
                    "stages".to_owned(),
                    serde_json::to_value(&details.stages).unwrap_or(Value::Null),
                );
            }
        }
        NodeDetails::Component(details) => {
            attributes.insert(
                "component_type".to_owned(),
                Value::String(details.component_type.clone()),
            );
        }
        NodeDetails::Messaging(details) => {
            attributes.insert(
                "transport".to_owned(),
                Value::String(details.transport.clone()),
            );
            attributes.insert("subject".to_owned(), Value::String(details.subject.clone()));
            attributes.insert(
                "declaring_scope".to_owned(),
                Value::String(details.declaring_scope.clone()),
            );
        }
        NodeDetails::Job(details) => {
            insert_optional_string(attributes, "schedule", details.schedule.as_ref());
            insert_optional_string(attributes, "queue", details.queue.as_ref());
        }
        NodeDetails::Schema(details) => {
            insert_optional_string(attributes, "dialect", details.dialect.as_ref());
            insert_optional_string(
                attributes,
                "logical_database",
                details.logical_database.as_ref(),
            );
            insert_optional_string(attributes, "namespace", details.namespace.as_ref());
        }
        NodeDetails::Query(details) => {
            insert_optional_string(attributes, "dialect", details.dialect.as_ref());
            insert_optional_string(attributes, "operation", details.operation.as_ref());
            insert_optional_string(attributes, "text_digest", details.text_digest.as_ref());
        }
        NodeDetails::Config(details) => {
            attributes.insert("format".to_owned(), Value::String(details.format.clone()));
            attributes.insert(
                "key_path".to_owned(),
                Value::String(details.key_path.clone()),
            );
        }
        NodeDetails::Database(details) => {
            attributes.insert(
                "logical_database".to_owned(),
                Value::String(details.logical_database.clone()),
            );
            insert_optional_string(
                attributes,
                "database_schema",
                details.database_schema.as_ref(),
            );
        }
    }
}

fn insert_raw_edge_details(attributes: &mut Map<String, Value>, details: &EdgeDetails) {
    match details {
        EdgeDetails::Call(details) => {
            attributes.insert(
                "dispatch".to_owned(),
                serde_json::to_value(details.dispatch).unwrap_or(Value::Null),
            );
            insert_optional_string(attributes, "receiver_type", details.receiver_type.as_ref());
            if let Some(count) = details.argument_count {
                attributes.insert("argument_count".to_owned(), Value::from(count));
            }
        }
        EdgeDetails::Route(details) => {
            attributes.insert(
                "stage".to_owned(),
                serde_json::to_value(details.stage).unwrap_or(Value::Null),
            );
            if let Some(position) = details.position {
                attributes.insert("position".to_owned(), Value::from(position));
            }
            insert_optional_string(attributes, "operation", details.operation.as_ref());
        }
        EdgeDetails::Messaging(details) => {
            attributes.insert(
                "transport".to_owned(),
                Value::String(details.transport.clone()),
            );
            attributes.insert("subject".to_owned(), Value::String(details.subject.clone()));
        }
        EdgeDetails::Schedule(details) => {
            insert_optional_string(attributes, "expression", details.expression.as_ref());
        }
        EdgeDetails::Mapping(details) => {
            attributes.insert(
                "mapping_kind".to_owned(),
                Value::String(details.mapping_kind.clone()),
            );
        }
    }
}

fn insert_optional_string(attributes: &mut Map<String, Value>, key: &str, value: Option<&String>) {
    if let Some(value) = value {
        attributes.insert(key.to_owned(), Value::String(value.clone()));
    }
}

fn normalize_file_inventory(files: &mut [FileRecord], root: &Path) -> Result<(), GraphError> {
    for file in files {
        file.path = portable_path(&file.path, root)?;
        file.id = file_id(&file.path);
        for diagnostic in &mut file.diagnostics {
            normalize_optional_anchor(&mut diagnostic.anchor, root)?;
        }
        for coverage in &mut file.coverage {
            normalize_optional_anchor(&mut coverage.anchor, root)?;
        }
    }
    Ok(())
}

fn normalize_logical_database_scope(
    attributes: &mut Map<String, Value>,
    root: &Path,
) -> Result<(), GraphError> {
    let Some(logical_database) = optional_string(attributes, "logical_database") else {
        return Ok(());
    };
    if !Path::new(&logical_database).is_absolute() {
        return Ok(());
    }
    let portable = portable_path(&logical_database, root)?;
    let qualified_prefix = format!("{logical_database}::");
    for key in ["name", "label", "qualified_name", "qualifiedName"] {
        let Some(value) = attributes.get(key).and_then(Value::as_str) else {
            continue;
        };
        let normalized = if value == logical_database {
            Some(portable.clone())
        } else {
            value
                .strip_prefix(&qualified_prefix)
                .map(|suffix| format!("{portable}::{suffix}"))
        };
        if let Some(normalized) = normalized {
            attributes.insert(key.to_owned(), Value::String(normalized));
        }
    }
    attributes.insert("logical_database".to_owned(), Value::String(portable));
    Ok(())
}

fn normalize_source_derived_scopes(
    attributes: &mut Map<String, Value>,
    root: &Path,
) -> Result<(), GraphError> {
    let Some(source_file) = optional_source_path(attributes, "source_file").or_else(|| {
        attributes
            .get("source_anchor")
            .or_else(|| attributes.get("sourceAnchor"))
            .or_else(|| attributes.get("anchor"))
            .and_then(Value::as_object)
            .and_then(|anchor| anchor.get("file"))
            .and_then(Value::as_str)
            .map(str::to_owned)
    }) else {
        return Ok(());
    };
    let portable_source = portable_path(&source_file, root)?;
    let source_path = Path::new(&source_file);
    let portable_source_path = Path::new(&portable_source);
    let dotted_components = |path: &Path| {
        path.with_extension("")
            .components()
            .filter_map(|component| component.as_os_str().to_str())
            .collect::<Vec<_>>()
            .join(".")
    };
    let dotted_trimmed = |path: &Path| {
        path.with_extension("")
            .to_string_lossy()
            .replace(['/', '\\'], ".")
            .trim_start_matches('.')
            .to_owned()
    };
    let source_scopes = [
        source_file.clone(),
        dotted_components(source_path),
        dotted_trimmed(source_path),
    ];
    let portable_scopes = [
        portable_source.clone(),
        dotted_components(portable_source_path),
        dotted_trimmed(portable_source_path),
    ];
    let dotted_roots = [dotted_components(root), dotted_trimmed(root)];

    for key in ["declaring_scope", "namespace"] {
        let Some(scope) = optional_string(attributes, key) else {
            continue;
        };
        if let Some(index) = source_scopes
            .iter()
            .position(|candidate| candidate == &scope)
        {
            attributes.insert(
                key.to_owned(),
                Value::String(portable_scopes[index].clone()),
            );
        } else if let Some(relative) = dotted_roots.iter().find_map(|dotted_root| {
            scope
                .strip_prefix(dotted_root)
                .and_then(|suffix| suffix.strip_prefix('.'))
        }) {
            attributes.insert(key.to_owned(), Value::String(relative.to_owned()));
        } else if Path::new(&scope).is_absolute() {
            attributes.insert(key.to_owned(), Value::String(portable_path(&scope, root)?));
        }
    }
    Ok(())
}

fn normalize_node(
    mut raw: RawNodeRecord,
    root: &Path,
    file_facts: &HashMap<String, PublishedFileFacts>,
    inferred_wiring_site: Option<&SourceAnchor>,
) -> Result<NodeRecord, GraphError> {
    normalize_logical_database_scope(&mut raw.attributes, root)?;
    normalize_source_derived_scopes(&mut raw.attributes, root)?;
    let raw_kind = raw
        .attributes
        .get("symbol_kind")
        .or_else(|| raw.attributes.get("type"))
        .and_then(Value::as_str)
        .or_else(|| {
            optional_source_path(&raw.attributes, "source_file")
                .is_none()
                .then_some("symbol")
        });
    let file_type = raw
        .attributes
        .get("file_type")
        .and_then(Value::as_str)
        .unwrap_or("code");
    let (kind, resource_kind) = map_node_kind(raw_kind, file_type)
        .ok_or_else(|| raw_error(&raw.id, "unknown raw node kind or file_type"))?;
    let name = required_any_string(&raw.attributes, &["name", "label"], &raw.id)?;
    let qualified_name = optional_any_string(&raw.attributes, &["qualified_name", "qualifiedName"])
        .unwrap_or_else(|| name.clone());
    let language = optional_any_string(&raw.attributes, &["language", "lang"]);
    let framework = optional_string(&raw.attributes, "framework");
    let source = raw_anchor(&raw.attributes, root, file_facts)?;
    let external_wiring_site = if source.is_none() {
        raw_origin_anchor(&raw.attributes, root, file_facts)?
            .or_else(|| inferred_wiring_site.cloned())
    } else {
        None
    };
    if source.is_none()
        && external_wiring_site.is_none()
        && optional_string(&raw.attributes, "extractor").as_deref()
            == Some("compass.graph.external-placeholder")
    {
        return Err(raw_error(
            &raw.id,
            "unresolved external placeholder requires an exact wiring site",
        ));
    }
    let source_path = match &source {
        Some(anchor) => anchor.file.clone(),
        None => optional_source_path(&raw.attributes, "source_file")
            .map(|path| portable_path(&path, root))
            .transpose()?
            .unwrap_or_default(),
    };
    let roles = raw
        .attributes
        .get("roles")
        .and_then(Value::as_array)
        .map(|roles| {
            roles
                .iter()
                .map(|role| {
                    role.as_str()
                        .and_then(map_role)
                        .ok_or_else(|| raw_error(&raw.id, "unknown node role"))
                })
                .collect::<Result<Vec<_>, _>>()
        })
        .transpose()?
        .unwrap_or_default();
    let primary_evidence = if kind == NodeKind::File
        && source.as_ref().is_some_and(|anchor| {
            anchor.start_byte == anchor.end_byte
                && file_facts
                    .get(&anchor.file)
                    .is_some_and(|file| file.byte_size == 0)
        }) {
        let provenance = Provenance {
            origin: EvidenceOrigin::Convention,
            extractor: optional_string(&raw.attributes, "extractor").unwrap_or_else(|| {
                optional_any_string(&raw.attributes, &["language", "lang"]).map_or_else(
                    || "compass.files.detect".to_owned(),
                    |language| format!("compass.languages.{language}"),
                )
            }),
            confidence: EvidenceConfidence::Exact,
            rule: Some("empty-file-inventory".to_owned()),
            anchors: source.clone().into_iter().collect(),
            wiring_site: None,
            score: None,
            candidates: Vec::new(),
        };
        provenance
            .validate()
            .map_err(|error| raw_error(&raw.id, &error.to_string()))?;
        provenance
    } else {
        normalize_provenance(
            &raw.attributes,
            source.clone().or_else(|| external_wiring_site.clone()),
            &raw.id,
            root,
            external_wiring_site
                .as_ref()
                .map(|_| "external-symbol-placeholder"),
            external_wiring_site.is_some(),
        )?
    };
    let mut evidence = vec![primary_evidence];
    if let Some(coalesced) = raw
        .attributes
        .get(COALESCED_NODE_EVIDENCE_ATTRIBUTE)
        .and_then(Value::as_array)
    {
        for value in coalesced {
            let provenance = serde_json::from_value::<Provenance>(value.clone())
                .map_err(|error| raw_error(&raw.id, &error.to_string()))?;
            evidence.push(provenance);
        }
        sort_dedup_serialized(&mut evidence);
    }
    let details = node_details(
        kind,
        resource_kind,
        &raw.attributes,
        &source_path,
        file_facts,
        &raw.id,
        root,
    )?;
    let id = node_identity(
        kind,
        &raw.attributes,
        language.as_deref(),
        framework.as_deref(),
        &source_path,
        &qualified_name,
        &raw.id,
        details.as_ref(),
        source.as_ref().or(external_wiring_site.as_ref()),
    )?;
    let diagnostics = external_wiring_site
        .as_ref()
        .map(|site| {
            vec![GraphDiagnostic {
                severity: DiagnosticSeverity::Warning,
                code: "unresolved_external_symbol".to_owned(),
                message: format!(
                    "external symbol {qualified_name:?} remains unresolved at its wiring site"
                ),
                anchor: Some(site.clone()),
                related_ids: vec![id.clone()],
            }]
        })
        .unwrap_or_default();
    let community = optional_u64(&raw.attributes, "community").map(|id| CommunityMetadata {
        id,
        label: optional_string(&raw.attributes, "community_name"),
        score: optional_f64(&raw.attributes, "community_score"),
        color: optional_string(&raw.attributes, "community_color"),
    });
    Ok(NodeRecord {
        id,
        kind,
        roles,
        name,
        qualified_name,
        language,
        framework,
        source,
        details,
        evidence,
        coverage: Vec::new(),
        diagnostics,
        community,
    })
}

fn normalize_edge(
    raw: RawEdgeRecord,
    source: &str,
    target: &str,
    index: usize,
    root: &Path,
    file_facts: &HashMap<String, PublishedFileFacts>,
) -> Result<EdgeRecord, GraphError> {
    let position = format!("edge[{index}]");
    let raw_relation = required_string(&raw.attributes, "relation", &position)?;
    let owner = format!("{position} {source} -[{raw_relation}]-> {target}");
    let (kind, alias_rule, heuristic) = map_edge_kind(&raw_relation)
        .ok_or_else(|| raw_error(&owner, &format!("unknown raw relation {raw_relation:?}")))?;
    let relationship_site = raw_anchor(&raw.attributes, root, file_facts)?;
    let normalization_rule = heuristic
        .then_some("indirect-call-resolution")
        .or(alias_rule);
    let mut evidence = Vec::new();
    let evidence_context = RawEdgeEvidenceContext {
        owner: &owner,
        root,
        file_facts,
        normalization_rule,
        heuristic_default: heuristic,
    };
    append_raw_edge_evidence(
        &mut evidence,
        &raw.attributes,
        relationship_site.clone(),
        &evidence_context,
    )?;
    if let Some(constituents) = raw
        .attributes
        .get("_coalesced_edge_evidence")
        .and_then(Value::as_array)
    {
        for constituent in constituents {
            let Some(attributes) = constituent.as_object() else {
                continue;
            };
            let constituent_site = raw_anchor(attributes, root, file_facts)?;
            append_raw_edge_evidence(
                &mut evidence,
                attributes,
                constituent_site,
                &evidence_context,
            )?;
        }
    }
    sort_dedup_serialized(&mut evidence);
    let occurrence_rule = raw_occurrence_rule(&raw.attributes);
    let id = edge_id(
        source,
        kind,
        target,
        relationship_site.as_ref(),
        occurrence_rule.as_ref().map(OccurrenceRule::as_str),
    );
    let details = (kind == EdgeKind::RoutesTo).then(|| {
        EdgeDetails::Route(RouteEdgeDetails {
            stage: match optional_string(&raw.attributes, "stage").as_deref() {
                Some("middleware") => RouteStage::Middleware,
                _ => RouteStage::Handler,
            },
            position: optional_u32(&raw.attributes, "position"),
            operation: optional_string(&raw.attributes, "operation"),
        })
    });
    Ok(EdgeRecord {
        key: id.clone(),
        id,
        source: source.to_owned(),
        target: target.to_owned(),
        kind,
        occurrence_rule,
        relationship_site,
        details,
        evidence,
        weight: optional_f64(&raw.attributes, "weight"),
        context: optional_string(&raw.attributes, "context"),
        deferred: raw
            .attributes
            .get("deferred")
            .and_then(Value::as_bool)
            .unwrap_or(false),
        diagnostics: Vec::new(),
    })
}

fn raw_occurrence_rule(attributes: &Map<String, Value>) -> Option<OccurrenceRule> {
    attributes
        .get(OCCURRENCE_RULE_ATTRIBUTE)
        .and_then(Value::as_str)
        .and_then(|rule| OccurrenceRule::new(rule.to_owned()))
        .or_else(|| {
            attributes
                .get(ENDPOINT_REWRITE_RULES_ATTRIBUTE)
                .and_then(Value::as_array)
                .filter(|rewrites| !rewrites.is_empty())
                .is_none()
                .then(|| {
                    attributes
                        .get("rule")
                        .and_then(Value::as_str)
                        .and_then(|rule| OccurrenceRule::new(rule.to_owned()))
                })
                .flatten()
        })
}

struct RawEdgeEvidenceContext<'a> {
    owner: &'a str,
    root: &'a Path,
    file_facts: &'a HashMap<String, PublishedFileFacts>,
    normalization_rule: Option<&'a str>,
    heuristic_default: bool,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawEndpointRewrite {
    rule: EndpointRewriteRule,
    score: f64,
    #[serde(default, rename = "_origin")]
    origin: Option<String>,
    #[serde(default)]
    confidence: Option<String>,
    #[serde(default)]
    extractor: Option<String>,
    #[serde(default)]
    source_file: Option<String>,
    #[serde(default)]
    source_location: Option<String>,
    #[serde(default)]
    source_anchor: Option<SourceAnchor>,
    #[serde(default)]
    line_start: Option<u32>,
    #[serde(default)]
    line_end: Option<u32>,
    #[serde(default)]
    column_start: Option<u32>,
    #[serde(default)]
    column_end: Option<u32>,
    #[serde(default)]
    start_byte: Option<u64>,
    #[serde(default)]
    end_byte: Option<u64>,
    #[serde(default)]
    candidates: Vec<ResolutionCandidate>,
}

impl RawEndpointRewrite {
    fn anchor_attributes(&self) -> Map<String, Value> {
        let mut attributes = Map::new();
        if let Some(value) = &self.source_file {
            attributes.insert("source_file".to_owned(), Value::String(value.clone()));
        }
        if let Some(value) = &self.source_location {
            attributes.insert("source_location".to_owned(), Value::String(value.clone()));
        }
        if let Some(value) = &self.source_anchor {
            attributes.insert(
                "source_anchor".to_owned(),
                serde_json::to_value(value).unwrap_or(Value::Null),
            );
        }
        for (key, value) in [
            ("line_start", self.line_start.map(u64::from)),
            ("line_end", self.line_end.map(u64::from)),
            ("column_start", self.column_start.map(u64::from)),
            ("column_end", self.column_end.map(u64::from)),
            ("start_byte", self.start_byte),
            ("end_byte", self.end_byte),
        ] {
            if let Some(value) = value {
                attributes.insert(key.to_owned(), Value::from(value));
            }
        }
        if !self.candidates.is_empty() {
            attributes.insert(
                "candidates".to_owned(),
                serde_json::to_value(&self.candidates).unwrap_or(Value::Null),
            );
        }
        attributes
    }
}

fn append_raw_edge_evidence(
    evidence: &mut Vec<Provenance>,
    attributes: &Map<String, Value>,
    relationship_site: Option<SourceAnchor>,
    context: &RawEdgeEvidenceContext<'_>,
) -> Result<(), GraphError> {
    evidence.push(normalize_provenance(
        attributes,
        relationship_site.clone(),
        context.owner,
        context.root,
        context.normalization_rule,
        context.heuristic_default,
    )?);
    append_raw_endpoint_rewrite_evidence(evidence, attributes, relationship_site, context)
}

fn append_raw_endpoint_rewrite_evidence(
    evidence: &mut Vec<Provenance>,
    attributes: &Map<String, Value>,
    relationship_site: Option<SourceAnchor>,
    context: &RawEdgeEvidenceContext<'_>,
) -> Result<(), GraphError> {
    let Some(raw_rewrites) = attributes.get(ENDPOINT_REWRITE_RULES_ATTRIBUTE) else {
        return Ok(());
    };
    let rewrites = raw_rewrites
        .as_array()
        .ok_or_else(|| raw_error(context.owner, "endpoint rewrite rules must be a JSON array"))?;
    for (index, raw_rewrite) in rewrites.iter().enumerate() {
        let record = format!("{} endpoint rewrite[{index}]", context.owner);
        let rewrite = serde_json::from_value::<RawEndpointRewrite>(raw_rewrite.clone())
            .map_err(|error| raw_error(&record, &format!("invalid endpoint rewrite: {error}")))?;
        if rewrite
            .origin
            .as_deref()
            .is_some_and(|origin| origin != "heuristic")
            || rewrite
                .confidence
                .as_deref()
                .is_some_and(|confidence| !matches!(confidence, "INFERRED" | "inferred"))
        {
            return Err(raw_error(
                &record,
                "endpoint rewrite cannot claim direct origin or exact confidence",
            ));
        }
        if !rewrite.score.is_finite() || !(0.0..=1.0).contains(&rewrite.score) {
            return Err(raw_error(
                &record,
                "endpoint rewrite score must be finite and between 0.0 and 1.0",
            ));
        }
        let rewrite_attributes = rewrite.anchor_attributes();
        let rewrite_site = raw_anchor(&rewrite_attributes, context.root, context.file_facts)?
            .or(relationship_site.clone());
        let Some(rewrite_site) =
            rewrite_site.filter(|site| site.is_valid() && site.start_byte < site.end_byte)
        else {
            return Err(raw_error(
                &record,
                "endpoint rewrite requires a valid non-empty exact wiring site",
            ));
        };
        let provenance = Provenance {
            origin: EvidenceOrigin::Heuristic,
            extractor: rewrite
                .extractor
                .or_else(|| optional_string(attributes, "extractor"))
                .unwrap_or_else(|| "compass.graph.endpoint-rewrite".to_owned()),
            confidence: EvidenceConfidence::Inferred,
            rule: Some(rewrite.rule.as_str().to_owned()),
            anchors: Vec::new(),
            wiring_site: Some(rewrite_site),
            score: Some(rewrite.score),
            candidates: normalize_candidates(&rewrite_attributes, context.root, &record)?,
        };
        provenance
            .validate()
            .map_err(|error| raw_error(&record, &error.to_string()))?;
        evidence.push(provenance);
    }
    Ok(())
}

fn normalize_provenance(
    attributes: &Map<String, Value>,
    anchor: Option<SourceAnchor>,
    record: &str,
    root: &Path,
    normalization_rule: Option<&str>,
    heuristic_default: bool,
) -> Result<Provenance, GraphError> {
    let raw_origin = optional_any_string(attributes, &["_origin", "origin"]);
    let confidence = match optional_string(attributes, "confidence").as_deref() {
        None if heuristic_default => EvidenceConfidence::Inferred,
        None | Some("EXTRACTED" | "exact") => EvidenceConfidence::Exact,
        Some("INFERRED" | "inferred") => EvidenceConfidence::Inferred,
        Some("AMBIGUOUS" | "ambiguous") => EvidenceConfidence::Ambiguous,
        Some(value) => {
            return Err(raw_error(
                record,
                &format!("unknown confidence value {value:?}"),
            ));
        }
    };
    let rule = optional_string(attributes, "rule")
        .or_else(|| normalization_rule.map(str::to_owned))
        .or_else(|| {
            (raw_origin.as_deref() == Some("semantic")).then(|| "semantic-extraction".to_owned())
        })
        .filter(|value| !value.trim().is_empty());
    if rule
        .as_deref()
        .and_then(EndpointRewriteRule::from_wire_name)
        .is_some()
    {
        return Err(raw_error(
            record,
            "closed endpoint rewrite rule names are reserved for _endpoint_rewrite_rules",
        ));
    }
    let origin = match raw_origin.as_deref() {
        None if heuristic_default => EvidenceOrigin::Heuristic,
        Some("ast") if heuristic_default => EvidenceOrigin::Heuristic,
        None if optional_string(attributes, "context").as_deref() == Some("scip") => {
            EvidenceOrigin::Artifact
        }
        None => EvidenceOrigin::Ast,
        Some("ast") => EvidenceOrigin::Ast,
        Some("config" | "configuration") => EvidenceOrigin::Config,
        Some("convention") => EvidenceOrigin::Convention,
        Some("artifact" | "scip") => EvidenceOrigin::Artifact,
        Some("semantic") => EvidenceOrigin::Heuristic,
        Some("heuristic") => EvidenceOrigin::Heuristic,
        Some(value) => {
            return Err(raw_error(
                record,
                &format!("unknown provenance origin {value:?}"),
            ));
        }
    };
    let extractor = if raw_origin.as_deref() == Some("semantic") {
        SEMANTIC_LAYER_EXTRACTOR.to_owned()
    } else {
        optional_string(attributes, "extractor").unwrap_or_else(|| {
            optional_any_string(attributes, &["language", "lang"]).map_or_else(
                || "compass.languages.unknown".to_owned(),
                |language| format!("compass.languages.{language}"),
            )
        })
    };
    let candidates = normalize_candidates(attributes, root, record)?;
    let mut provenance = Provenance {
        origin,
        extractor,
        confidence,
        rule,
        anchors: Vec::new(),
        wiring_site: None,
        score: optional_f64(attributes, "confidence_score")
            .or_else(|| optional_f64(attributes, "score")),
        candidates,
    };
    if origin == EvidenceOrigin::Heuristic {
        provenance.wiring_site = anchor;
    } else if let Some(anchor) = anchor {
        provenance.anchors.push(anchor);
    }
    provenance
        .validate()
        .map_err(|error| raw_error(record, &error.to_string()))?;
    Ok(provenance)
}

fn normalize_candidates(
    attributes: &Map<String, Value>,
    root: &Path,
    record: &str,
) -> Result<Vec<ResolutionCandidate>, GraphError> {
    let Some(candidates) = attributes.get("candidates") else {
        return Ok(Vec::new());
    };
    let mut candidates = serde_json::from_value::<Vec<ResolutionCandidate>>(candidates.clone())
        .map_err(|error| raw_error(record, &format!("invalid candidates: {error}")))?;
    for candidate in &mut candidates {
        normalize_optional_anchor(&mut candidate.anchor, root)?;
    }
    sort_dedup_candidates(&mut candidates);
    Ok(candidates)
}

fn remap_provenance_candidates(evidence: &mut Vec<Provenance>, id_remap: &HashMap<String, String>) {
    for provenance in evidence.iter_mut() {
        for candidate in &mut provenance.candidates {
            if let Some(published) = id_remap.get(&candidate.node_id) {
                candidate.node_id.clone_from(published);
            }
        }
        sort_dedup_candidates(&mut provenance.candidates);
    }
    sort_dedup_serialized(evidence);
}

fn sort_dedup_candidates(candidates: &mut Vec<ResolutionCandidate>) {
    candidates.sort_by(|left, right| {
        left.node_id
            .cmp(&right.node_id)
            .then_with(|| {
                route_candidate_confidence_rank(left.confidence)
                    .cmp(&route_candidate_confidence_rank(right.confidence))
            })
            .then_with(|| match (left.score, right.score) {
                (Some(left), Some(right)) => right.total_cmp(&left),
                (Some(_), None) => std::cmp::Ordering::Less,
                (None, Some(_)) => std::cmp::Ordering::Greater,
                (None, None) => std::cmp::Ordering::Equal,
            })
            .then_with(|| right.anchor.is_some().cmp(&left.anchor.is_some()))
            .then_with(|| serialized(left).cmp(&serialized(right)))
    });
    candidates.dedup_by(|left, right| left.node_id == right.node_id);
}

fn route_stage_details(
    attributes: &Map<String, Value>,
    record: &str,
    root: &Path,
) -> Result<Vec<RouteStageDetails>, GraphError> {
    let Some(stages) = attributes.get("stages") else {
        return Ok(Vec::new());
    };
    let mut stages = serde_json::from_value::<Vec<RouteStageDetails>>(stages.clone())
        .map_err(|error| raw_error(record, &format!("invalid route stages: {error}")))?;
    for stage in &mut stages {
        for candidate in &mut stage.candidates {
            normalize_optional_anchor(&mut candidate.anchor, root)?;
        }
    }
    Ok(stages)
}

fn map_node_kind(
    raw_kind: Option<&str>,
    file_type: &str,
) -> Option<(NodeKind, Option<ResourceKind>)> {
    let resource = match file_type {
        "document" => Some(ResourceKind::Document),
        "paper" => Some(ResourceKind::Paper),
        "image" => Some(ResourceKind::Image),
        "concept" => Some(ResourceKind::Concept),
        "rationale" => Some(ResourceKind::Rationale),
        _ => None,
    };
    if resource.is_some() {
        return Some((NodeKind::Resource, resource));
    }
    let kind = match raw_kind? {
        "file" => NodeKind::File,
        "module" => NodeKind::Module,
        "package" => NodeKind::Package,
        "namespace" => NodeKind::Namespace,
        "class" => NodeKind::Class,
        "struct" | "record" => NodeKind::Struct,
        "interface" => NodeKind::Interface,
        "trait" => NodeKind::Trait,
        "protocol" => NodeKind::Protocol,
        "enum" => NodeKind::Enum,
        "enum_member" | "enum_constant" => NodeKind::EnumMember,
        "type_alias" | "alias" | "type" => NodeKind::TypeAlias,
        "function" => NodeKind::Function,
        "method" | "destructor" => NodeKind::Method,
        "constructor" => NodeKind::Constructor,
        "property" => NodeKind::Property,
        "field" => NodeKind::Field,
        "variable" => NodeKind::Variable,
        "constant" => NodeKind::Constant,
        "parameter" => NodeKind::Parameter,
        "import" => NodeKind::Import,
        "export" => NodeKind::Export,
        "macro" => NodeKind::Macro,
        "annotation" => NodeKind::Annotation,
        "route" => NodeKind::Route,
        "component" => NodeKind::Component,
        "event" => NodeKind::Event,
        "message" => NodeKind::Message,
        "topic" => NodeKind::Topic,
        "queue" => NodeKind::Queue,
        "job" => NodeKind::Job,
        "resource" => NodeKind::Resource,
        "schema" => NodeKind::Schema,
        "query" => NodeKind::Query,
        "migration" => NodeKind::Migration,
        "config_key" | "config" => NodeKind::ConfigKey,
        "database" => NodeKind::Database,
        "database_schema" => NodeKind::DatabaseSchema,
        "database_table" | "table" => NodeKind::DatabaseTable,
        "database_view" | "view" => NodeKind::DatabaseView,
        "database_column" | "column" => NodeKind::DatabaseColumn,
        "database_index" | "index" => NodeKind::DatabaseIndex,
        "database_constraint" | "constraint" => NodeKind::DatabaseConstraint,
        "database_procedure" | "procedure" => NodeKind::DatabaseProcedure,
        "database_trigger" | "trigger" => NodeKind::DatabaseTrigger,
        _ => return None,
    };
    Some((kind, None))
}

/// Resolve an extractor relation name through the complete v1 alias table.
#[must_use]
pub fn canonical_edge_kind(raw: &str) -> Option<EdgeKind> {
    map_edge_kind(raw).map(|(kind, _, _)| kind)
}

/// Resolve raw edge source locations with the same byte-range semantics used
/// by v1 publication, reading each referenced source file at most once.
#[must_use]
pub fn canonical_raw_edge_sites(edges: &[RawEdgeRecord], root: &Path) -> Vec<Option<SourceAnchor>> {
    let mut file_facts = HashMap::new();
    for edge in edges {
        let Some(source_file) = optional_source_path(&edge.attributes, "source_file") else {
            continue;
        };
        let Ok(portable) = portable_path(&source_file, root) else {
            continue;
        };
        if file_facts.contains_key(&portable) {
            continue;
        }
        let Ok(bytes) = fs::read(root.join(&portable)) else {
            continue;
        };
        file_facts.insert(portable, PublishedFileFacts::from_bytes(&bytes, false));
    }
    edges
        .iter()
        .map(|edge| {
            raw_anchor(&edge.attributes, root, &file_facts)
                .ok()
                .flatten()
        })
        .collect()
}

fn map_edge_kind(raw: &str) -> Option<(EdgeKind, Option<&'static str>, bool)> {
    let mapped = match raw {
        "contains" => (EdgeKind::Contains, None, false),
        "calls" => (EdgeKind::Calls, None, false),
        "imports" => (EdgeKind::Imports, None, false),
        "exports" => (EdgeKind::Exports, None, false),
        "extends" => (EdgeKind::Extends, None, false),
        "implements" => (EdgeKind::Implements, None, false),
        "references" => (EdgeKind::References, None, false),
        "type_of" => (EdgeKind::TypeOf, None, false),
        "returns" => (EdgeKind::Returns, None, false),
        "instantiates" => (EdgeKind::Instantiates, None, false),
        "overrides" => (EdgeKind::Overrides, None, false),
        "decorates" => (EdgeKind::Decorates, None, false),
        "routes_to" => (EdgeKind::RoutesTo, None, false),
        "reads" => (EdgeKind::Reads, None, false),
        "writes" => (EdgeKind::Writes, None, false),
        "aliases" => (EdgeKind::Aliases, None, false),
        "registers" => (EdgeKind::Registers, None, false),
        "handles" => (EdgeKind::Handles, None, false),
        "publishes" => (EdgeKind::Publishes, None, false),
        "subscribes" => (EdgeKind::Subscribes, None, false),
        "produces" => (EdgeKind::Produces, None, false),
        "consumes" => (EdgeKind::Consumes, None, false),
        "schedules" => (EdgeKind::Schedules, None, false),
        "triggers" => (EdgeKind::Triggers, None, false),
        "tests" => (EdgeKind::Tests, None, false),
        "depends_on" => (EdgeKind::DependsOn, None, false),
        "documents" => (EdgeKind::Documents, None, false),
        "maps_to" => (EdgeKind::MapsTo, None, false),
        "imports_from" => (EdgeKind::Imports, Some("raw-relation:imports_from"), false),
        "re_exports" => (EdgeKind::Exports, Some("raw-relation:re_exports"), false),
        "inherits" => (EdgeKind::Extends, None, false),
        "indirect_call" => (EdgeKind::Calls, Some("indirect-call-resolution"), true),
        "reads_from" => (EdgeKind::Reads, None, false),
        "references_constant" | "uses_static_prop" | "uses" | "scip_ref" | "scip_def" => {
            (EdgeKind::References, None, false)
        }
        "scip_typed" => (EdgeKind::TypeOf, None, false),
        "scip_impl" => (EdgeKind::Implements, None, false),
        "rationale_for" => (EdgeKind::Documents, None, false),
        "configures" => (EdgeKind::DependsOn, None, false),
        "case_of" | "defines" | "method" => (EdgeKind::Contains, None, false),
        "embeds" => (EdgeKind::Contains, Some("embedded-member"), false),
        "mixes_in" => (EdgeKind::Implements, Some("mixin-contract"), false),
        _ => return None,
    };
    Some(mapped)
}

fn node_details(
    kind: NodeKind,
    resource_kind: Option<ResourceKind>,
    attributes: &Map<String, Value>,
    source_path: &str,
    file_facts: &HashMap<String, PublishedFileFacts>,
    record: &str,
    root: &Path,
) -> Result<Option<NodeDetails>, GraphError> {
    let details = match kind {
        NodeKind::File => {
            let file = file_facts.get(source_path).ok_or_else(|| {
                raw_error(record, "file node has no matching file inventory record")
            })?;
            Some(NodeDetails::File(FileNodeDetails {
                content_digest: file.content_digest.clone(),
                byte_size: file.byte_size,
                generated: file.generated,
            }))
        }
        NodeKind::Import | NodeKind::Export => {
            Some(NodeDetails::ImportExport(ImportExportNodeDetails {
                specifier: required_any_string(attributes, &["specifier", "label"], record)?,
                imported_name: optional_string(attributes, "imported_name"),
                local_name: optional_string(attributes, "local_name"),
                type_only: attributes
                    .get("type_only")
                    .and_then(Value::as_bool)
                    .unwrap_or(false),
            }))
        }
        NodeKind::Route => Some(NodeDetails::Route(RouteNodeDetails {
            operation: required_string(attributes, "operation", record)?,
            path: required_any_string(attributes, &["path", "route_path"], record)?,
            original_path: optional_string(attributes, "original_path"),
            declaring_scope: required_string(attributes, "declaring_scope", record)?,
            resolution: match optional_string(attributes, "resolution").as_deref() {
                Some("ambiguous") => ResolutionState::Ambiguous,
                Some("unresolved") => ResolutionState::Unresolved,
                _ => ResolutionState::Exact,
            },
            middleware_count: optional_u32(attributes, "middleware_count").unwrap_or(0),
            stages: route_stage_details(attributes, record, root)?,
        })),
        NodeKind::Resource => Some(NodeDetails::Resource(ResourceNodeDetails {
            resource_kind: resource_kind.unwrap_or(ResourceKind::Document),
            uri: optional_string(attributes, "uri"),
            media_type: optional_string(attributes, "media_type"),
        })),
        NodeKind::Event | NodeKind::Message | NodeKind::Topic | NodeKind::Queue => {
            Some(NodeDetails::Messaging(MessagingNodeDetails {
                transport: required_string(attributes, "transport", record)?,
                subject: required_string(attributes, "subject", record)?,
                declaring_scope: required_string(attributes, "declaring_scope", record)?,
            }))
        }
        NodeKind::Schema => Some(NodeDetails::Schema(SchemaNodeDetails {
            dialect: optional_string(attributes, "dialect"),
            logical_database: optional_string(attributes, "logical_database"),
            namespace: optional_string(attributes, "namespace"),
        })),
        NodeKind::Query => Some(NodeDetails::Query(QueryNodeDetails {
            dialect: optional_string(attributes, "dialect"),
            operation: optional_string(attributes, "operation"),
            text_digest: optional_string(attributes, "text_digest"),
        })),
        NodeKind::ConfigKey => Some(NodeDetails::Config(ConfigNodeDetails {
            format: required_string(attributes, "format", record)?,
            key_path: required_any_string(attributes, &["key_path", "qualified_name"], record)?,
        })),
        NodeKind::Database
        | NodeKind::DatabaseSchema
        | NodeKind::DatabaseTable
        | NodeKind::DatabaseView
        | NodeKind::DatabaseColumn
        | NodeKind::DatabaseIndex
        | NodeKind::DatabaseConstraint
        | NodeKind::DatabaseProcedure
        | NodeKind::DatabaseTrigger => Some(NodeDetails::Database(DatabaseNodeDetails {
            logical_database: required_string(attributes, "logical_database", record)?,
            database_schema: optional_string(attributes, "database_schema"),
        })),
        NodeKind::Component => Some(NodeDetails::Component(
            compass_model::code_graph::ComponentNodeDetails {
                component_type: required_any_string(
                    attributes,
                    &["component_type", "type"],
                    record,
                )?,
            },
        )),
        NodeKind::Job => Some(NodeDetails::Job(
            compass_model::code_graph::JobNodeDetails {
                schedule: optional_string(attributes, "schedule"),
                queue: optional_string(attributes, "queue"),
            },
        )),
        NodeKind::Module
        | NodeKind::Package
        | NodeKind::Namespace
        | NodeKind::Class
        | NodeKind::Struct
        | NodeKind::Interface
        | NodeKind::Trait
        | NodeKind::Protocol
        | NodeKind::Enum
        | NodeKind::EnumMember
        | NodeKind::TypeAlias
        | NodeKind::Function
        | NodeKind::Method
        | NodeKind::Constructor
        | NodeKind::Property
        | NodeKind::Field
        | NodeKind::Variable
        | NodeKind::Constant
        | NodeKind::Parameter
        | NodeKind::Macro
        | NodeKind::Annotation
        | NodeKind::Migration => Some(NodeDetails::Symbol(SymbolNodeDetails {
            signature: optional_string(attributes, "signature"),
            modifiers: attributes
                .get("modifiers")
                .and_then(Value::as_array)
                .map(|values| {
                    values
                        .iter()
                        .filter_map(Value::as_str)
                        .map(str::to_owned)
                        .collect()
                })
                .unwrap_or_default(),
            overload_discriminator: optional_string(attributes, "overload_discriminator"),
            declaring_type: optional_string(attributes, "declaring_type"),
            signature_digest: optional_string(attributes, "signature_hash"),
            implementation_digest: optional_string(attributes, "implementation_hash"),
            source_digest: optional_string(attributes, "source_hash"),
        })),
    };
    Ok(details)
}

#[allow(clippy::too_many_arguments)]
fn node_identity(
    kind: NodeKind,
    attributes: &Map<String, Value>,
    language: Option<&str>,
    framework: Option<&str>,
    source_path: &str,
    qualified_name: &str,
    record: &str,
    details: Option<&NodeDetails>,
    identity_site: Option<&SourceAnchor>,
) -> Result<String, GraphError> {
    let id = match kind {
        NodeKind::File => file_id(source_path),
        NodeKind::Route => {
            let Some(NodeDetails::Route(route)) = details else {
                return Err(raw_error(record, "route details are missing"));
            };
            let target_namespace = route
                .stages
                .iter()
                .find(|stage| stage.stage == RouteStage::Handler)
                .map(|stage| {
                    if stage.reference.is_empty() {
                        stage.target.as_deref().unwrap_or_default()
                    } else {
                        stage.reference.as_str()
                    }
                })
                .unwrap_or_default();
            route_id(
                framework.ok_or_else(|| raw_error(record, "route framework is missing"))?,
                source_path,
                &route.operation,
                &route.path,
                &route.declaring_scope,
                target_namespace,
            )
        }
        NodeKind::Event | NodeKind::Message | NodeKind::Topic | NodeKind::Queue => {
            let Some(NodeDetails::Messaging(message)) = details else {
                return Err(raw_error(record, "messaging details are missing"));
            };
            messaging_id(
                kind,
                &message.transport,
                &message.subject,
                &message.declaring_scope,
            )
        }
        NodeKind::Database
        | NodeKind::DatabaseSchema
        | NodeKind::DatabaseTable
        | NodeKind::DatabaseView
        | NodeKind::DatabaseColumn
        | NodeKind::DatabaseIndex
        | NodeKind::DatabaseConstraint
        | NodeKind::DatabaseProcedure
        | NodeKind::DatabaseTrigger => {
            let Some(NodeDetails::Database(database)) = details else {
                return Err(raw_error(record, "database details are missing"));
            };
            database_entity_id(
                kind,
                &database.logical_database,
                database.database_schema.as_deref().unwrap_or_default(),
                qualified_name,
            )
        }
        NodeKind::Resource
            if matches!(
                details,
                Some(NodeDetails::Resource(ResourceNodeDetails {
                    resource_kind: ResourceKind::Rationale,
                    ..
                }))
            ) =>
        {
            let positional_name = identity_site.map_or_else(
                || qualified_name.to_owned(),
                |site| format!("{qualified_name}@{}:{}", site.start_byte, site.end_byte),
            );
            domain_id(kind, source_path, &positional_name)
        }
        NodeKind::Import | NodeKind::Export => {
            let binding = match details {
                Some(NodeDetails::ImportExport(details)) => format!(
                    "{}:{}:{}",
                    details.imported_name.as_deref().unwrap_or_default(),
                    details.local_name.as_deref().unwrap_or_default(),
                    details.type_only
                ),
                _ => String::new(),
            };
            symbol_id(
                language.unwrap_or("unknown"),
                source_path,
                kind,
                qualified_name,
                &binding,
            )
        }
        NodeKind::Job
        | NodeKind::Resource
        | NodeKind::Schema
        | NodeKind::Query
        | NodeKind::ConfigKey => {
            let namespace =
                optional_string(attributes, "namespace").unwrap_or_else(|| source_path.to_owned());
            domain_id(kind, &namespace, qualified_name)
        }
        _ => {
            let unresolved_scope;
            let identity_source = if source_path.is_empty() {
                unresolved_scope = optional_string(attributes, "external_identity_scope")
                    .filter(|scope| !scope.trim().is_empty())
                    .unwrap_or_else(|| {
                        identity_site.map_or_else(
                            || format!("unresolved:{record}"),
                            |site| format!("{}#{}:{}", site.file, site.start_byte, site.end_byte),
                        )
                    });
                unresolved_scope.as_str()
            } else {
                source_path
            };
            let overload = optional_string(attributes, "overload_discriminator");
            let lexical_owner =
                optional_any_string(attributes, &["lexical_owner", "declaring_scope"]);
            let disambiguator = match (lexical_owner, overload) {
                (Some(owner), Some(overload)) => format!("{owner}::{overload}"),
                (Some(owner), None) => owner,
                (None, Some(overload)) => overload,
                (None, None) => String::new(),
            };
            symbol_id(
                language.unwrap_or("unknown"),
                identity_source,
                kind,
                qualified_name,
                &disambiguator,
            )
        }
    };
    Ok(id)
}

fn raw_anchor(
    attributes: &Map<String, Value>,
    root: &Path,
    file_facts: &HashMap<String, PublishedFileFacts>,
) -> Result<Option<SourceAnchor>, GraphError> {
    if let Some(value) = attributes
        .get("source_anchor")
        .or_else(|| attributes.get("sourceAnchor"))
        .or_else(|| attributes.get("anchor"))
    {
        let mut anchor =
            serde_json::from_value::<SourceAnchor>(value.clone()).map_err(|error| {
                raw_error(
                    "source anchor",
                    &format!("invalid structured anchor: {error}"),
                )
            })?;
        anchor.file = portable_path(&anchor.file, root)?;
        return Ok(Some(anchor));
    }
    let Some(source_file) = optional_source_path(attributes, "source_file") else {
        return Ok(None);
    };
    let source_file = portable_path(&source_file, root)?;
    let start_line = optional_u32(attributes, "line_start")
        .or_else(|| source_location_line(attributes))
        .unwrap_or(1);
    let end_line = optional_u32(attributes, "line_end").unwrap_or(start_line);
    let start_column = optional_u32(attributes, "column_start").unwrap_or(0);
    let end_column = optional_u32(attributes, "column_end");
    let explicit = optional_u64(attributes, "start_byte").zip(optional_u64(attributes, "end_byte"));
    let derived = file_facts
        .get(&source_file)
        .and_then(|file| file.byte_range(start_line, start_column, end_line, end_column));
    let Some((start_byte, end_byte, end_column)) = explicit
        .map(|(start, end)| (start, end, end_column.unwrap_or(start_column)))
        .or(derived)
    else {
        return Ok(None);
    };
    Ok(Some(SourceAnchor {
        file: source_file,
        start_byte,
        end_byte,
        start_line,
        start_column,
        end_line,
        end_column,
    }))
}

fn raw_origin_anchor(
    attributes: &Map<String, Value>,
    root: &Path,
    file_facts: &HashMap<String, PublishedFileFacts>,
) -> Result<Option<SourceAnchor>, GraphError> {
    let Some(origin_file) = optional_source_path(attributes, "origin_file") else {
        return Ok(None);
    };
    let origin_file = portable_path(&origin_file, root)?;
    Ok(file_facts
        .get(&origin_file)
        .and_then(|file| file.full_file_anchor(origin_file)))
}

fn normalize_optional_anchor(
    anchor: &mut Option<SourceAnchor>,
    root: &Path,
) -> Result<(), GraphError> {
    if let Some(anchor) = anchor {
        anchor.file = portable_path(&anchor.file, root)?;
    }
    Ok(())
}

fn portable_path(path: &str, root: &Path) -> Result<String, GraphError> {
    let candidate = Path::new(path);
    let relative = if candidate.is_absolute() {
        candidate
            .strip_prefix(root)
            .map(Path::to_path_buf)
            .or_else(|_| {
                let candidate = fs::canonicalize(candidate).map_err(|_| ())?;
                let root = fs::canonicalize(root).map_err(|_| ())?;
                candidate
                    .strip_prefix(root)
                    .map(Path::to_path_buf)
                    .map_err(|_| ())
            })
            .map_err(|()| {
                raw_error(
                    path,
                    "absolute source path is outside the declared repository root",
                )
            })?
    } else {
        candidate.to_path_buf()
    };
    let normalized = normalize_repository_path(&relative.to_string_lossy());
    if normalized.is_empty() || normalized == ".." || normalized.starts_with("../") {
        return Err(raw_error(path, "source path escapes the repository root"));
    }
    Ok(normalized)
}

fn map_role(value: &str) -> Option<NodeRole> {
    match value {
        "controller" => Some(NodeRole::Controller),
        "route_handler" => Some(NodeRole::RouteHandler),
        "middleware" => Some(NodeRole::Middleware),
        "service" => Some(NodeRole::Service),
        "resolver" => Some(NodeRole::Resolver),
        "consumer" => Some(NodeRole::Consumer),
        "producer" => Some(NodeRole::Producer),
        "subscriber" => Some(NodeRole::Subscriber),
        "repository" => Some(NodeRole::Repository),
        "model" => Some(NodeRole::Model),
        "test" => Some(NodeRole::Test),
        "fixture" => Some(NodeRole::Fixture),
        "generated" => Some(NodeRole::Generated),
        _ => None,
    }
}

fn required_string(
    attributes: &Map<String, Value>,
    key: &str,
    record: &str,
) -> Result<String, GraphError> {
    optional_string(attributes, key)
        .filter(|value| !value.trim().is_empty())
        .ok_or_else(|| raw_error(record, &format!("required string {key:?} is missing")))
}

fn required_any_string(
    attributes: &Map<String, Value>,
    keys: &[&str],
    record: &str,
) -> Result<String, GraphError> {
    optional_any_string(attributes, keys)
        .filter(|value| !value.trim().is_empty())
        .ok_or_else(|| {
            raw_error(
                record,
                &format!("one of the required strings {keys:?} is missing"),
            )
        })
}

fn optional_any_string(attributes: &Map<String, Value>, keys: &[&str]) -> Option<String> {
    keys.iter().find_map(|key| optional_string(attributes, key))
}

fn optional_string(attributes: &Map<String, Value>, key: &str) -> Option<String> {
    attributes
        .get(key)
        .and_then(Value::as_str)
        .map(str::to_owned)
}

fn optional_source_path(attributes: &Map<String, Value>, key: &str) -> Option<String> {
    optional_string(attributes, key).filter(|path| !path.trim().is_empty())
}

fn optional_u64(attributes: &Map<String, Value>, key: &str) -> Option<u64> {
    attributes.get(key).and_then(Value::as_u64)
}

fn optional_u32(attributes: &Map<String, Value>, key: &str) -> Option<u32> {
    optional_u64(attributes, key).and_then(|value| u32::try_from(value).ok())
}

fn optional_f64(attributes: &Map<String, Value>, key: &str) -> Option<f64> {
    attributes.get(key).and_then(Value::as_f64)
}

fn source_location_line(attributes: &Map<String, Value>) -> Option<u32> {
    attributes
        .get("source_location")
        .and_then(Value::as_str)
        .and_then(|value| value.strip_prefix('L'))
        .and_then(|value| value.split(':').next())
        .and_then(|value| value.parse().ok())
}

fn coverage_key(record: &CoverageRecord) -> (&str, &str, Option<&str>) {
    (
        record.capability.as_str(),
        record.producer.as_str(),
        record.file_id.as_deref(),
    )
}

struct PublishedFileFacts {
    content_digest: String,
    byte_size: u64,
    generated: bool,
    line_starts: Vec<u64>,
}

impl PublishedFileFacts {
    fn from_bytes(bytes: &[u8], generated: bool) -> Self {
        let mut line_starts = vec![0];
        line_starts.extend(bytes.iter().enumerate().filter_map(|(index, byte)| {
            if *byte == b'\n' {
                u64::try_from(index).ok().map(|value| value + 1)
            } else {
                None
            }
        }));
        Self {
            content_digest: sha256_prefixed(bytes),
            byte_size: bytes.len() as u64,
            generated,
            line_starts,
        }
    }

    fn full_file_anchor(&self, file: String) -> Option<SourceAnchor> {
        (self.byte_size > 0).then(|| {
            let end_line_index = self.line_starts.len().saturating_sub(1);
            let end_line_start = self.line_starts[end_line_index];
            SourceAnchor {
                file,
                start_byte: 0,
                end_byte: self.byte_size,
                start_line: 1,
                start_column: 0,
                end_line: u32::try_from(end_line_index.saturating_add(1)).unwrap_or(u32::MAX),
                end_column: u32::try_from(self.byte_size.saturating_sub(end_line_start))
                    .unwrap_or(u32::MAX),
            }
        })
    }

    fn byte_range(
        &self,
        start_line: u32,
        start_column: u32,
        end_line: u32,
        end_column: Option<u32>,
    ) -> Option<(u64, u64, u32)> {
        let start_index = usize::try_from(start_line.checked_sub(1)?).ok()?;
        let end_index = usize::try_from(end_line.checked_sub(1)?).ok()?;
        let line_start = *self.line_starts.get(start_index)?;
        let next_line = self
            .line_starts
            .get(end_index.saturating_add(1))
            .copied()
            .unwrap_or(self.byte_size);
        let end_line_start = *self.line_starts.get(end_index)?;
        let maximum_column = u32::try_from(next_line.saturating_sub(end_line_start)).ok()?;
        let end_column = end_column.unwrap_or(maximum_column).min(maximum_column);
        let start_byte = line_start.saturating_add(u64::from(start_column));
        let end_byte = end_line_start.saturating_add(u64::from(end_column));
        (start_byte <= end_byte && end_byte <= self.byte_size)
            .then_some((start_byte, end_byte, end_column))
    }
}

fn published_file_facts(
    evidence: &BuildEvidence,
) -> Result<HashMap<String, PublishedFileFacts>, GraphError> {
    evidence
        .files
        .iter()
        .map(|file| {
            let absolute = evidence.repository_root.join(&file.path);
            let bytes = fs::read(&absolute).map_err(|source| GraphError::Read {
                path: absolute,
                source,
            })?;
            if bytes.len() as u64 != file.byte_size
                || sha256_prefixed(&bytes) != file.content_digest
            {
                return Err(raw_error(
                    &file.path,
                    "file inventory digest or byte size does not match the source tree",
                ));
            }
            Ok((
                file.path.clone(),
                PublishedFileFacts::from_bytes(&bytes, file.generated),
            ))
        })
        .collect()
}

fn sha256_prefixed(bytes: &[u8]) -> String {
    format!("sha256:{:x}", Sha256::digest(bytes))
}

fn schema_fingerprint() -> String {
    let mut vocabulary = BTreeMap::new();
    vocabulary.insert(
        "nodes",
        [
            "file",
            "module",
            "package",
            "namespace",
            "class",
            "struct",
            "interface",
            "trait",
            "protocol",
            "enum",
            "enum_member",
            "type_alias",
            "function",
            "method",
            "constructor",
            "property",
            "field",
            "variable",
            "constant",
            "parameter",
            "import",
            "export",
            "macro",
            "annotation",
            "route",
            "component",
            "event",
            "message",
            "topic",
            "queue",
            "job",
            "resource",
            "schema",
            "query",
            "migration",
            "config_key",
            "database",
            "database_schema",
            "database_table",
            "database_view",
            "database_column",
            "database_index",
            "database_constraint",
            "database_procedure",
            "database_trigger",
        ]
        .as_slice(),
    );
    vocabulary.insert(
        "edges",
        [
            "contains",
            "calls",
            "imports",
            "exports",
            "extends",
            "implements",
            "references",
            "type_of",
            "returns",
            "instantiates",
            "overrides",
            "decorates",
            "routes_to",
            "reads",
            "writes",
            "aliases",
            "registers",
            "handles",
            "publishes",
            "subscribes",
            "produces",
            "consumes",
            "schedules",
            "triggers",
            "tests",
            "depends_on",
            "documents",
            "maps_to",
        ]
        .as_slice(),
    );
    let mut digest = Sha256::new();
    for (category, values) in vocabulary {
        digest.update((category.len() as u64).to_le_bytes());
        digest.update(category.as_bytes());
        for value in values {
            digest.update((value.len() as u64).to_le_bytes());
            digest.update(value.as_bytes());
        }
    }
    format!("sha256:{:x}", digest.finalize())
}

fn raw_error(record: &str, detail: &str) -> GraphError {
    GraphError::RawNormalization {
        record: record.to_owned(),
        detail: detail.to_owned(),
    }
}
