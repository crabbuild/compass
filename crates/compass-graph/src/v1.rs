use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::fs;
use std::path::{Path, PathBuf};

use compass_languages::{Extraction, RawEdgeRecord, RawNodeRecord};
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
    ENDPOINT_REWRITE_RULES_ATTRIBUTE, EndpointRewriteRule, EvidenceConfidence, EvidenceOrigin,
    OCCURRENCE_RULE_ATTRIBUTE, OccurrenceRule, Provenance, ResolutionCandidate, ResolutionState,
    SourceAnchor, TRUSTED_EDGE_RECORD_ATTRIBUTE,
};
use compass_model::{GraphError, validate_code_graph};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use sha2::{Digest, Sha256};

const TRUSTED_NODE_RECORD: &str = "_compass_v1_node_record";
const TRUSTED_EDGE_RECORD: &str = TRUSTED_EDGE_RECORD_ATTRIBUTE;
const TRUSTED_GRAPH_COVERAGE: &str = "_compass_v1_graph_coverage";
const TRUSTED_GRAPH_DIAGNOSTICS: &str = "_compass_v1_graph_diagnostics";
const COALESCED_EDGE_EVIDENCE: &str = "_coalesced_edge_evidence";

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
        for attributes in extraction
            .nodes
            .iter()
            .map(|node| &node.attributes)
            .chain(extraction.edges.iter().map(|edge| &edge.attributes))
        {
            if let Some(path) = optional_source_path(attributes, "source_file")
                .or_else(|| optional_source_path(attributes, "origin_file"))
            {
                paths.insert(portable_path(&path, &repository_root)?);
            }
        }

        let mut source_tree = Sha256::new();
        let mut files = Vec::with_capacity(paths.len());
        for path in paths {
            let absolute = repository_root.join(&path);
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
                language: None,
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
            diagnostics: Vec::new(),
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
                language: None,
                content_digest: sha256_prefixed(&bytes),
                byte_size: bytes.len() as u64,
                generated: item.status == ExtractionStatus::Generated,
                extraction_status: item.status,
                extractor_versions: vec![format!(
                    "compass-languages/{}",
                    env!("CARGO_PKG_VERSION")
                )],
                coverage: Vec::new(),
                diagnostics: Vec::new(),
            };
            if let Some(existing) = self.files.iter_mut().find(|file| file.path == path) {
                *existing = record;
            } else {
                self.files.push(record);
            }
            self.coverage.push(CoverageRecord {
                capability: "file_inventory".to_owned(),
                producer: "compass.files.detect".to_owned(),
                status: match item.status {
                    ExtractionStatus::Extracted => CoverageStatus::Complete,
                    ExtractionStatus::Partial => CoverageStatus::Partial,
                    ExtractionStatus::Unsupported => CoverageStatus::Unsupported,
                    ExtractionStatus::Excluded => CoverageStatus::Excluded,
                    ExtractionStatus::ParseFailure => CoverageStatus::Failed,
                    ExtractionStatus::Generated | ExtractionStatus::Binary => {
                        CoverageStatus::Indeterminate
                    }
                },
                file_id: Some(file_id(&path)),
                reason: item.reason,
                anchor: None,
            });
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
        .map(|path| portable_path(&path, root).map(|path| file_id(&path)))
        .transpose()
}

/// Publish resolved raw facts as a validated, deterministic Compass graph v1 document.
pub fn normalize_v1(
    mut extraction: Extraction,
    mut evidence: BuildEvidence,
) -> Result<GraphDocument, GraphError> {
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

    let mut id_remap = HashMap::with_capacity(extraction.nodes.len());
    let mut nodes = BTreeMap::new();
    for raw in extraction.nodes {
        if id_remap.contains_key(&raw.id) {
            return Err(raw_error(
                &raw.id,
                "duplicate raw node ID cannot be resolved deterministically",
            ));
        }
        let node = match raw.attributes.get(TRUSTED_NODE_RECORD) {
            Some(value) => serde_json::from_value::<NodeRecord>(value.clone())
                .map_err(|error| raw_error(&raw.id, &error.to_string()))?,
            None => normalize_node(
                raw.clone(),
                &evidence.repository_root,
                &file_facts,
                stub_wiring_sites.get(&raw.id),
            )?,
        };
        id_remap.insert(raw.id, node.id.clone());
        if let Some(existing) = nodes.get_mut(&node.id) {
            merge_normalized_node(existing, node)?;
        } else {
            nodes.insert(node.id.clone(), node);
        }
    }
    for node in nodes.values_mut() {
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
    }

    let mut links = BTreeMap::new();
    for (index, raw) in extraction.edges.into_iter().enumerate() {
        let source = id_remap.get(&raw.source).ok_or_else(|| {
            raw_error(
                &format!("edge[{index}]"),
                &format!("source {} does not match a raw node", raw.source),
            )
        })?;
        let target = id_remap.get(&raw.target).ok_or_else(|| {
            raw_error(
                &format!("edge[{index}]"),
                &format!("target {} does not match a raw node", raw.target),
            )
        })?;
        let edge = match raw.attributes.get(TRUSTED_EDGE_RECORD).cloned() {
            Some(value) => normalize_trusted_edge(
                raw,
                value,
                source,
                target,
                index,
                &evidence.repository_root,
                &file_facts,
            )?,
            None => normalize_edge(
                raw,
                source,
                target,
                index,
                &evidence.repository_root,
                &file_facts,
            )?,
        };
        if edge.source == edge.target && edge.kind != EdgeKind::Calls {
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
            let source_kind = nodes
                .get(&edge.source)
                .map(|node| node.kind)
                .unwrap_or(NodeKind::Variable);
            let target_kind = nodes
                .get(&edge.target)
                .map(|node| node.kind)
                .unwrap_or(NodeKind::Variable);
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
            merge_normalized_edge(existing, edge)?;
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
        let Some(stage) = route_details
            .stages
            .iter_mut()
            .find(|stage| stage.position == position && stage.stage == edge_details.stage)
        else {
            continue;
        };
        stage.target = Some(edge.target.clone());
        if stage.resolution == ResolutionState::Exact
            && let [candidate] = stage.candidates.as_mut_slice()
        {
            candidate.node_id.clone_from(&edge.target);
        }
    }

    let mut nodes = nodes.into_values().collect::<Vec<_>>();
    let mut links = links.into_values().collect::<Vec<_>>();
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
    evidence
        .coverage
        .sort_by(|left, right| coverage_key(left).cmp(&coverage_key(right)));
    evidence.diagnostics.sort_by(|left, right| {
        (left.code.as_str(), left.message.as_str())
            .cmp(&(right.code.as_str(), right.message.as_str()))
    });

    let document = GraphDocument {
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
    validate_code_graph(&document)?;
    Ok(document)
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
    }
    let split_ids = scopes
        .iter()
        .filter(|(_, values)| values.len() > 1)
        .map(|(id, values)| {
            let clones = values
                .iter()
                .map(|scope| {
                    let digest = Sha256::digest(scope.as_bytes());
                    (scope.clone(), format!("{id}#wiring-{digest:x}"))
                })
                .collect::<BTreeMap<_, _>>();
            (id.clone(), clones)
        })
        .collect::<HashMap<_, _>>();
    if split_ids.is_empty() {
        return Ok(());
    }

    let mut expanded = Vec::with_capacity(extraction.nodes.len() + split_ids.len());
    for node in std::mem::take(&mut extraction.nodes) {
        let Some(clones) = split_ids.get(&node.id) else {
            expanded.push(node);
            continue;
        };
        for (scope, clone_id) in clones {
            let mut clone = node.clone();
            clone.id.clone_from(clone_id);
            clone
                .attributes
                .insert("declaring_scope".to_owned(), Value::String(scope.clone()));
            expanded.push(clone);
        }
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
        anchor.start_byte.to_string(),
        anchor.end_byte.to_string(),
    ]
    .join("\u{1f}")
}

fn anchor_key(anchor: &SourceAnchor) -> (&str, u64, u64) {
    (&anchor.file, anchor.start_byte, anchor.end_byte)
}

fn merge_normalized_node(
    existing: &mut NodeRecord,
    mut duplicate: NodeRecord,
) -> Result<(), GraphError> {
    if existing.kind != duplicate.kind
        || existing.name != duplicate.name
        || existing.qualified_name != duplicate.qualified_name
        || existing.language != duplicate.language
        || existing.framework != duplicate.framework
        || existing.source != duplicate.source
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
    existing.details = deterministic_option(existing.details.take(), duplicate.details);
    existing.community = deterministic_option(existing.community.take(), duplicate.community);
    Ok(())
}

fn merge_normalized_edge(
    existing: &mut EdgeRecord,
    mut duplicate: EdgeRecord,
) -> Result<(), GraphError> {
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
    existing.evidence.append(&mut duplicate.evidence);
    sort_dedup_serialized(&mut existing.evidence);
    existing.diagnostics.append(&mut duplicate.diagnostics);
    sort_dedup_serialized(&mut existing.diagnostics);
    existing.details = deterministic_option(existing.details.take(), duplicate.details);
    existing.weight = deterministic_option(existing.weight.take(), duplicate.weight);
    existing.context = deterministic_option(existing.context.take(), duplicate.context);
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
    normalize_v1(extraction, evidence)
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

fn normalize_node(
    raw: RawNodeRecord,
    root: &Path,
    file_facts: &HashMap<String, PublishedFileFacts>,
    inferred_wiring_site: Option<&SourceAnchor>,
) -> Result<NodeRecord, GraphError> {
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
    let evidence = vec![normalize_provenance(
        &raw.attributes,
        source.clone().or_else(|| external_wiring_site.clone()),
        &raw.id,
        root,
        external_wiring_site
            .as_ref()
            .map(|_| "external-symbol-placeholder"),
        external_wiring_site.is_some(),
    )?];
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
        diagnostics: Vec::new(),
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
    let extractor = optional_string(attributes, "extractor").unwrap_or_else(|| {
        optional_any_string(attributes, &["language", "lang"]).map_or_else(
            || "compass.languages.unknown".to_owned(),
            |language| format!("compass.languages.{language}"),
        )
    });
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
    candidates.sort_by(|left, right| left.node_id.cmp(&right.node_id));
    Ok(candidates)
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
        "variable" | "symbol" => NodeKind::Variable,
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
            overload_discriminator: optional_any_string(
                attributes,
                &["overload_discriminator", "signature_hash"],
            ),
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
            route_id(
                framework.ok_or_else(|| raw_error(record, "route framework is missing"))?,
                source_path,
                &route.operation,
                &route.path,
                &route.declaring_scope,
                identity_site,
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
                unresolved_scope = identity_site.map_or_else(
                    || format!("unresolved:{record}"),
                    |site| format!("{}#{}:{}", site.file, site.start_byte, site.end_byte),
                );
                unresolved_scope.as_str()
            } else {
                source_path
            };
            let overload =
                optional_any_string(attributes, &["overload_discriminator", "signature_hash"]);
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
        candidate.strip_prefix(root).map_err(|_| {
            raw_error(
                path,
                "absolute source path is outside the declared repository root",
            )
        })?
    } else {
        candidate
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
            let mut line_starts = vec![0];
            line_starts.extend(bytes.iter().enumerate().filter_map(|(index, byte)| {
                if *byte == b'\n' {
                    u64::try_from(index).ok().map(|value| value + 1)
                } else {
                    None
                }
            }));
            Ok((
                file.path.clone(),
                PublishedFileFacts {
                    content_digest: file.content_digest.clone(),
                    byte_size: file.byte_size,
                    generated: file.generated,
                    line_starts,
                },
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
