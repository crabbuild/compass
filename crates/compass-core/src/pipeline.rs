use std::borrow::Cow;
use std::cmp::Ordering;
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::fs;
use std::io::{BufReader, Write};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use ahash::{AHashMap, AHashSet};
use compass_files::{
    BuildGuard, BuildScope, Cache, CacheOptions, DetectOptions, Detection, IgnorePolicy, Manifest,
    ManifestKind, detect, write_atomic_with_digest, write_json_atomic, write_text_atomic,
};
use compass_graph::{
    BuildEvidence, ClusterOptions, EntityTiebreaker, GRAPH_DIAGNOSTICS_EXTENSION,
    GRAPH_JSON_DELTA_MAX_SOURCE_BYTES, GRAPH_SNAPSHOT_MAX_OBJECTS,
    GRAPH_SNAPSHOT_SELECTOR_SCHEMA_V1, GraphSnapshotBuilder, GraphSnapshotGcStats,
    IncrementalClusterLimits, InferenceLevel, InventoryEvidence, PublicationOmissions,
    SnapshotSelector, SourceDigest, apply_inference_level,
    build_owned_with_tiebreaker_at_inference as build_document, canonical_edge_kind,
    canonical_raw_edge_sites, cluster_incremental, deduped_node_count, extraction_from_v1,
    garbage_collect_graph_snapshots, graph_insights, graph_snapshot_needs_gc,
    label_communities_by_hub, normalize_document_v1_with_evidence_best_effort_owned_at_inference,
    normalize_document_v1_with_inventory_and_source_digests_best_effort_owned_at_inference,
    normalize_document_v1_with_inventory_best_effort_at_inference, score_communities,
    write_canonical_graph_json, write_fact_neutral_graph_json_delta_prevalidated,
};
use compass_languages::{
    BindingFact, DeclarationFact, EXTRACTION_QUALITY_EXTENSION, EXTRACTION_QUALITY_PARTIAL,
    EXTRACTION_QUALITY_REASON_EXTENSION, Engine, Extraction, ExtractorKind,
    FRAMEWORK_PROJECT_EVIDENCE_EXTENSION, OccurrenceFact, ProjectEvidenceIndex, RawCall,
    RawEdgeRecord, RawFrameworkFact, RawNodeRecord, Registry, RelationshipCandidate,
    ResolutionConstraint, ScopeFact, SemanticEvidenceBatch, file_stem, make_id,
};
use compass_model::code_graph::{
    CommunityMetadata, CoverageRecord, DiagnosticSeverity, ExtractionStatus, FileNodeDetails,
    GraphDiagnostic, GraphDocument as V1GraphDocument, NodeDetails, NodeKind,
};
use compass_model::provenance::{
    COALESCED_NODE_EVIDENCE_ATTRIBUTE, CONSUME_INCREMENTAL_ENDPOINT_REMAP_ATTRIBUTE,
    EndpointRewriteEvidence, EndpointRewriteRule, EvidenceOrigin, OCCURRENCE_RULE_ATTRIBUTE,
    Provenance, SEMANTIC_LAYER_EXTRACTOR, SourceAnchor, TRUSTED_EDGE_RECORD_ATTRIBUTE,
    append_endpoint_rewrite_evidence,
};
use compass_model::{EdgeRecord, GraphDocument, NodeRecord};
use compass_output::{
    DetectionSummary, FreshnessBasis, FreshnessStatus, GraphViewModel, HtmlOptions,
    OrientationHealth, OutputError, PublicationStatus, ReportOptions, TokenCost, agent_orientation,
    graph_view_model_document, render_agent_report_markdown, render_orientation_json, write_html,
};
use compass_resolve::{
    ResolutionAdmission, apply_program_projection, collect_program_projection_sites,
    merge_decl_def_classes_if_needed, merge_decl_def_classes_if_needed_changed,
    resolve_prevalidated_owned_with_root_at_inference,
};
use compass_store::{
    GRAPH_SCHEMA_V1, STORE_FILE_NAME, STORE_REF_FILE_NAME, SqliteStore, StoreRef,
    local_sqlite_store_path,
};
use rayon::prelude::*;
use serde::ser::{Error as SerdeError, SerializeMap, SerializeSeq, Serializer};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};
use sha2::{Digest, Sha256};

use crate::build_state::{
    ArtifactSeal, BUILD_STATE_FILE, BuildProfile, BuildState, SavedStats, load_verified,
};
use crate::program::{
    PreparedSyntaxInput, ProgramBuild, build_program, load_current_program, program_artifact_count,
    write_program,
};
use crate::raw_guard::enforce_incomplete_raw_guard;

/// Default upper bound for one source file entering the parser pipeline.
///
/// Syntax and semantic extractors can retain several multiples of the source
/// size while building records. Callers may raise this bound explicitly for
/// repositories with intentionally large generated sources.
pub const DEFAULT_MAX_SOURCE_BYTES: u64 = 16 * 1024 * 1024;
const SEMANTIC_MARKER_FILE: &str = "semantic-marker.json";
const PIPELINE_RAYON_WORKER_CAP: usize = 12;
const PARALLEL_AST_FACT_DIGEST_MIN_FILES: usize = 32;
const STORE_SNAPSHOT_EXCLUSIONS: [&str; 3] =
    [STORE_FILE_NAME, "store.sqlite3-wal", "store.sqlite3-shm"];
const ROOT_ARTIFACTS: [&str; 7] = [
    "GRAPH_REPORT.md",
    "orientation.json",
    "graph-overview.json",
    "graph.html",
    "manifest.json",
    "program.json",
    "graph.json",
];

#[derive(Clone, Debug)]
pub struct BuildOptions {
    pub root: PathBuf,
    pub scan_filesystem: bool,
    pub output_root: Option<PathBuf>,
    /// Explicit repository-private cache root used by exact history builds.
    pub cache_root: Option<PathBuf>,
    pub force: bool,
    /// Reuse validated AST and Program cache entries while retaining force
    /// authorization for output replacement.
    pub reuse_cache_on_force: bool,
    pub no_cluster: bool,
    pub no_viz: bool,
    /// Durable graph storage published with the canonical JSON artifact.
    ///
    /// JSON is always published as the portable authority. SQLite is the
    /// default query index for bounded, large-graph reads; `--store json`
    /// opts out when only the portable artifact is wanted.
    pub graph_storage: GraphStorage,
    /// Maximum inferred relationship class admitted to the published graph.
    ///
    /// Extraction caches retain complete evidence regardless of this policy;
    /// changing the level deterministically republishes a coherent subgraph.
    pub inference_level: InferenceLevel,
    pub gitignore: bool,
    pub ignore_policy: IgnorePolicy,
    pub extra_excludes: Vec<String>,
    pub scope: BuildScope,
    pub resolution: f64,
    pub exclude_hubs: Option<f64>,
    pub google_workspace: bool,
    /// Restrict structural extraction to files classified as code.
    ///
    /// This is the core representation of the CLI's `--code-only` profile;
    /// keeping it in the build profile prevents a document-inclusive output
    /// from being reused for a code-only build.
    pub code_only: bool,
    /// Enable deterministic Program IR analysis and `program.json` output.
    pub program_analysis: bool,
    /// Explicit offline program evidence artifacts, in addition to `index.scip`.
    pub program_artifacts: Vec<PathBuf>,
    /// Resource limits for offline program artifacts.
    pub program_artifact_limits: compass_program::ArtifactLimits,
    /// Maximum number of worker threads used by the deterministic AST stages.
    /// `None` uses the bounded host CPU count in a build-local Rayon pool once
    /// enough files are missing to amortize parser-table residency.
    pub max_workers: Option<usize>,
    /// Maximum size of one source file admitted to AST and Program analysis.
    ///
    /// Oversized files remain in the inventory with partial coverage and an
    /// explicit reason; they are not read into memory.
    pub max_source_bytes: u64,
    /// Override the commit recorded in update artifacts.
    ///
    /// This is primarily useful for reproducible builds and compatibility
    /// tests whose oracle and native halves must share one source revision.
    pub built_at_commit: Option<String>,
    pub purpose: BuildPurpose,
    /// Detection already validated by an init preview.
    pub precomputed_detection: Option<Detection>,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum GraphStorage {
    Json,
    #[default]
    Sqlite,
}

impl GraphStorage {
    const fn publishes_store(self) -> bool {
        matches!(self, Self::Sqlite)
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum BuildPurpose {
    #[default]
    Update,
    Extract,
}

const OUTPUT_STATS_FILE: &str = "output-stats.json";
const AST_FACT_DIGESTS_FILE: &str = "ast-fact-digests.json";
const AST_FACT_DIGESTS_SCHEMA: &str = "compass.ast-fact-digests/3";
const MAX_AST_FACT_DIGEST_ENTRIES: usize = 100_000;
const MAX_AST_FACT_DIGEST_FILE_BYTES: usize = 16 * 1024 * 1024;
const GRAPH_OVERVIEW_FILE: &str = "graph-overview.json";
const GRAPH_OVERVIEW_SCHEMA: &str = "compass.graph-overview/2";
const GRAPH_OVERVIEW_NODE_LIMIT: isize = 5_000;
const MAX_INCREMENTAL_REMAP_DIAGNOSTICS: usize = 100;
const INCREMENTAL_REMAP_DROP_DIAGNOSTIC: &str = "dropped_incremental_remap_without_wiring_site";
const INCREMENTAL_REMAP_TRUNCATION_DIAGNOSTIC: &str =
    "incremental_remap_without_wiring_site_truncated";
#[derive(Clone, Debug, Deserialize, Serialize)]
struct OutputStats {
    graph_bytes: u64,
    nodes: usize,
    edges: usize,
    communities: usize,
    clustered: bool,
    #[serde(default)]
    omitted_nodes: usize,
    #[serde(default)]
    omitted_edges: usize,
    #[serde(default)]
    identity_collisions: usize,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
struct AstFactDigestState {
    schema: String,
    profile_digest: String,
    project_evidence_digest: String,
    entries: BTreeMap<String, String>,
}

impl AstFactDigestState {
    fn is_bounded(&self) -> bool {
        self.schema == AST_FACT_DIGESTS_SCHEMA
            && !self.profile_digest.is_empty()
            && !self.project_evidence_digest.is_empty()
            && self.entries.len() <= MAX_AST_FACT_DIGEST_ENTRIES
            && self.entries.iter().all(|(path, digest)| {
                path.len() <= 4_096 && digest.len() <= 128 && !path.contains(['\\', '\0'])
            })
    }
}

struct FactNeutralExtractionBatch {
    extractions: Vec<Extraction>,
    source_digests: BTreeMap<String, SourceDigest>,
    empty_files: Vec<PathBuf>,
}

/// Release parser/extractor over-allocation before a result crosses the worker
/// boundary. Extractors grow vectors and JSON maps incrementally, so a large
/// source can otherwise retain roughly the next power-of-two capacity for
/// every node, edge, and evidence container while the project-wide resolver
/// keeps all file facts alive.
///
/// This only changes allocation capacity. It deliberately does not remove or
/// normalize any fact, because the portable AST cache and the resolver must
/// see the exact same evidence as the non-compacted path.
fn compact_extraction(extraction: &mut Extraction) {
    for node in &mut extraction.nodes {
        compact_json_map(&mut node.attributes);
    }
    for edge in &mut extraction.edges {
        compact_json_map(&mut edge.attributes);
    }
    for value in &mut extraction.hyperedges {
        compact_json_value(value);
    }
    if let Some(calls) = extraction.raw_calls.as_mut() {
        for call in &mut *calls {
            compact_json_map(&mut call.extensions);
        }
        compact_vec(calls);
    }
    for fact in &mut extraction.framework_facts {
        match fact {
            compass_languages::RawFrameworkFact::Route(route) => {
                compact_vec(&mut route.middleware_references);
                compact_json_map(&mut route.detail);
            }
            compass_languages::RawFrameworkFact::Domain(domain) => {
                compact_json_map(&mut domain.detail);
            }
            compass_languages::RawFrameworkFact::Annotation(annotation) => {
                compact_json_map(&mut annotation.arguments);
                compact_json_map(&mut annotation.detail);
            }
        }
    }
    if let Some(evidence) = extraction.semantic_evidence.as_mut() {
        compact_vec(&mut evidence.adapter.capabilities);
        compact_vec(&mut evidence.declarations);
        compact_vec(&mut evidence.scopes);
        compact_vec(&mut evidence.bindings);
        compact_vec(&mut evidence.occurrences);
        compact_vec(&mut evidence.candidates);
        compact_vec(&mut evidence.diagnostics);
    }
    compact_json_map(&mut extraction.extensions);
    compact_vec(&mut extraction.nodes);
    compact_vec(&mut extraction.edges);
    compact_vec(&mut extraction.hyperedges);
    compact_vec(&mut extraction.framework_facts);
}

/// Shrink a vector only when it retained a meaningful amount of growth slack.
///
/// Extractors commonly reserve a small amount beyond the final length while
/// parsing a file. Reallocating those tight buffers costs CPU without making a
/// useful difference to the project-wide working set. A 25% slack threshold
/// preserves the memory reduction for vectors that grew substantially while
/// making the per-file compaction pass cheap for ordinary source files.
fn compact_vec<T>(values: &mut Vec<T>) {
    let useful_capacity = values
        .len()
        .saturating_add(values.len() / 4)
        .saturating_add(1);
    if values.capacity() > useful_capacity {
        values.shrink_to_fit();
    }
}

fn compact_json_map(map: &mut serde_json::Map<String, Value>) {
    for value in map.values_mut() {
        compact_json_value(value);
    }
    // serde_json intentionally keeps its backing map implementation private,
    // so rebuild genuinely large maps with an exact initial capacity. Medium
    // maps are left alone because rebuilding them costs more than their bounded
    // slack on the common per-node/per-edge path.
    if map.len() < 32 {
        return;
    }
    let values = std::mem::take(map);
    let mut compacted = serde_json::Map::with_capacity(values.len());
    for (key, value) in values {
        compacted.insert(key, value);
    }
    *map = compacted;
}

fn compact_json_value(value: &mut Value) {
    match value {
        Value::Array(values) => {
            for value in values.iter_mut() {
                compact_json_value(value);
            }
            compact_vec(values);
        }
        Value::Object(map) => compact_json_map(map),
        Value::Null | Value::Bool(_) | Value::Number(_) | Value::String(_) => {}
    }
}

impl OutputStats {
    const fn omissions(&self) -> PublicationOmissions {
        PublicationOmissions {
            nodes: self.omitted_nodes,
            edges: self.omitted_edges,
            identity_collisions: self.identity_collisions,
            examples_omitted: 0,
        }
    }
}

impl BuildOptions {
    #[must_use]
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self {
            root: root.into(),
            scan_filesystem: true,
            output_root: None,
            cache_root: None,
            force: false,
            reuse_cache_on_force: false,
            no_cluster: false,
            no_viz: false,
            graph_storage: GraphStorage::default(),
            inference_level: InferenceLevel::default(),
            gitignore: true,
            ignore_policy: IgnorePolicy::CurrentCheckout,
            extra_excludes: Vec::new(),
            scope: BuildScope::default(),
            resolution: 1.0,
            exclude_hubs: None,
            google_workspace: false,
            code_only: false,
            program_analysis: false,
            program_artifacts: Vec::new(),
            program_artifact_limits: compass_program::ArtifactLimits::default(),
            // Large builds use a bounded local pool. Keeping this unset uses
            // the measured memory/throughput default; CLI callers can provide
            // an explicit bound when their environment needs another tradeoff.
            max_workers: None,
            max_source_bytes: DEFAULT_MAX_SOURCE_BYTES,
            built_at_commit: None,
            purpose: BuildPurpose::Update,
            precomputed_detection: None,
        }
    }
}

const fn cache_reuse_enabled(force: bool, reuse_cache_on_force: bool) -> bool {
    !force || reuse_cache_on_force
}

const fn prior_published_graph_input_enabled(force: bool) -> bool {
    !force
}

fn prior_semantic_layer_required(read_prior_published_graph: bool, output_dir: &Path) -> bool {
    read_prior_published_graph && output_dir.join(SEMANTIC_MARKER_FILE).is_file()
}

fn load_ast_fact_digest_state(output_dir: &Path) -> Option<AstFactDigestState> {
    let path = output_dir.join(AST_FACT_DIGESTS_FILE);
    let bytes = fs::read(path).ok()?;
    if bytes.len() > MAX_AST_FACT_DIGEST_FILE_BYTES {
        return None;
    }
    let state = serde_json::from_slice::<AstFactDigestState>(&bytes).ok()?;
    state.is_bounded().then_some(state)
}

fn write_ast_fact_digest_state(
    output_dir: &Path,
    state: &AstFactDigestState,
) -> Result<(), CoreError> {
    if !state.is_bounded() {
        remove_if_exists(&output_dir.join(AST_FACT_DIGESTS_FILE))?;
        return Ok(());
    }
    let encoded = serde_json::to_vec(state).map_err(|source| CoreError::SerializeExtraction {
        path: output_dir.join(AST_FACT_DIGESTS_FILE),
        source,
    })?;
    if encoded.len() > MAX_AST_FACT_DIGEST_FILE_BYTES {
        remove_if_exists(&output_dir.join(AST_FACT_DIGESTS_FILE))?;
        return Ok(());
    }
    write_json_atomic(output_dir.join(AST_FACT_DIGESTS_FILE), state, false).map_err(CoreError::from)
}

fn relative_fact_path(path: &Path, root: &Path) -> String {
    path.strip_prefix(root)
        .unwrap_or(path)
        .to_string_lossy()
        .replace('\\', "/")
}

fn build_profile_digest(profile: &BuildProfile, output_dir: &Path) -> Result<String, CoreError> {
    let bytes = serde_json::to_vec(profile).map_err(|source| CoreError::SerializeExtraction {
        path: output_dir.join(AST_FACT_DIGESTS_FILE),
        source,
    })?;
    let mut digest = Sha256::new();
    digest.update(b"compass.ast-fact-digests/profile/3");
    digest.update([0]);
    for component in [
        compass_model::code_graph::CODE_GRAPH_SCHEMA_V1,
        compass_graph::V1_PUBLICATION_SEMANTICS_VERSION,
        compass_languages::EXTRACTION_SEMANTICS_VERSION,
        compass_files::AST_CACHE_VERSION,
    ] {
        digest.update((component.len() as u64).to_le_bytes());
        digest.update(component.as_bytes());
    }
    digest.update((bytes.len() as u64).to_le_bytes());
    digest.update(bytes);
    Ok(format!("sha256:{:x}", digest.finalize()))
}

fn project_evidence_digest(
    project_evidence: &ProjectEvidenceIndex,
    sources: &[PathBuf],
    root: &Path,
) -> String {
    let mut entries = BTreeMap::new();
    for source in sources {
        let key = relative_fact_path(source, root);
        entries.insert(key, project_evidence.fingerprint_for(source).to_owned());
    }
    let mut digest = Sha256::new();
    for (path, fingerprint) in entries {
        digest.update((path.len() as u64).to_le_bytes());
        digest.update(path.as_bytes());
        digest.update((fingerprint.len() as u64).to_le_bytes());
        digest.update(fingerprint.as_bytes());
    }
    format!("sha256:{:x}", digest.finalize())
}

/// Hash the semantic extraction facts for one source file while ignoring the
/// file-record envelope. A trailing comment or whitespace edit may legitimately
/// change the file inventory node and digest without changing any symbol or
/// relationship fact. Source anchors on actual symbols remain part of this
/// digest, so edits that shift or alter graph facts still take the full path.
const FILE_ENVELOPE_ATTRIBUTES: &[&str] = &[
    "source_anchor",
    "source_location",
    "start_byte",
    "end_byte",
    "line_start",
    "line_end",
    "column_start",
    "column_end",
    "content_digest",
    "byte_size",
    "generated",
];

struct ExtractionDigestWriter(Sha256);

impl Write for ExtractionDigestWriter {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        self.0.update(bytes);
        Ok(bytes.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

/// Serialize JSON object keys in lexical order for digesting.
///
/// `serde_json` is configured with `preserve_order` in this workspace. That
/// is useful when round-tripping user-facing JSON, but it means an equivalent
/// map can acquire a different byte representation after a cache load. Fact
/// digests describe semantic extraction, so map insertion order must not be
/// part of their identity.
fn compare_fact_values(left: &Value, right: &Value) -> Ordering {
    fn kind(value: &Value) -> u8 {
        match value {
            Value::Null => 0,
            Value::Bool(_) => 1,
            Value::Number(_) => 2,
            Value::String(_) => 3,
            Value::Array(_) => 4,
            Value::Object(_) => 5,
        }
    }

    let kind_order = kind(left).cmp(&kind(right));
    if kind_order != Ordering::Equal {
        return kind_order;
    }
    match (left, right) {
        (Value::Null, Value::Null) => Ordering::Equal,
        (Value::Bool(left), Value::Bool(right)) => left.cmp(right),
        (Value::Number(left), Value::Number(right)) => left.to_string().cmp(&right.to_string()),
        (Value::String(left), Value::String(right)) => left.cmp(right),
        (Value::Array(left), Value::Array(right)) => left
            .iter()
            .zip(right)
            .map(|(left, right)| compare_fact_values(left, right))
            .find(|order| *order != Ordering::Equal)
            .unwrap_or_else(|| left.len().cmp(&right.len())),
        (Value::Object(left), Value::Object(right)) => compare_fact_maps(left, right),
        _ => Ordering::Equal,
    }
}

fn compare_fact_maps(left: &Map<String, Value>, right: &Map<String, Value>) -> Ordering {
    let mut left_entries = left.iter().collect::<Vec<_>>();
    let mut right_entries = right.iter().collect::<Vec<_>>();
    left_entries.sort_unstable_by(|left, right| left.0.cmp(right.0));
    right_entries.sort_unstable_by(|left, right| left.0.cmp(right.0));
    left_entries
        .iter()
        .zip(&right_entries)
        .map(|((left_key, left_value), (right_key, right_value))| {
            left_key
                .cmp(right_key)
                .then_with(|| compare_fact_values(left_value, right_value))
        })
        .find(|order| *order != Ordering::Equal)
        .unwrap_or_else(|| left_entries.len().cmp(&right_entries.len()))
}

fn normalized_call_text(value: &Option<Option<String>>) -> Option<&str> {
    value.as_ref().and_then(|value| value.as_deref())
}

struct FactDigestValue<'a>(&'a Value);

impl Serialize for FactDigestValue<'_> {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        match self.0 {
            Value::Null => serializer.serialize_unit(),
            Value::Bool(value) => serializer.serialize_bool(*value),
            Value::Number(value) => value.serialize(serializer),
            Value::String(value) => serializer.serialize_str(value),
            Value::Array(values) => {
                let mut sequence = serializer.serialize_seq(Some(values.len()))?;
                for value in values {
                    sequence.serialize_element(&Self(value))?;
                }
                sequence.end()
            }
            Value::Object(values) => {
                let mut entries = values.iter().collect::<Vec<_>>();
                entries.sort_unstable_by(|left, right| left.0.cmp(right.0));
                let mut map = serializer.serialize_map(Some(entries.len()))?;
                for (key, value) in entries {
                    map.serialize_entry(key, &Self(value))?;
                }
                map.end()
            }
        }
    }
}

struct FactDigestNode<'a> {
    node: &'a RawNodeRecord,
}

impl Serialize for FactDigestNode<'_> {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        let is_file = self.node.string("symbol_kind") == "file";
        let mut attributes = self
            .node
            .attributes
            .iter()
            .filter(|(key, _)| !(is_file && FILE_ENVELOPE_ATTRIBUTES.contains(&key.as_str())))
            .collect::<Vec<_>>();
        attributes.sort_unstable_by(|left, right| left.0.cmp(right.0));
        let mut map = serializer.serialize_map(Some(attributes.len() + 1))?;
        map.serialize_entry("id", &self.node.id)?;
        for (key, value) in attributes {
            map.serialize_entry(key, &FactDigestValue(value))?;
        }
        map.end()
    }
}

struct FactDigestNodes<'a> {
    nodes: &'a [RawNodeRecord],
}

impl Serialize for FactDigestNodes<'_> {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        let mut nodes = self.nodes.iter().collect::<Vec<_>>();
        nodes.sort_unstable_by(|left, right| left.id.cmp(&right.id));
        let mut sequence = serializer.serialize_seq(Some(self.nodes.len()))?;
        for node in nodes {
            sequence.serialize_element(&FactDigestNode { node })?;
        }
        sequence.end()
    }
}

struct FactDigestEdge<'a> {
    edge: &'a RawEdgeRecord,
}

impl Serialize for FactDigestEdge<'_> {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        let mut attributes = self.edge.attributes.iter().collect::<Vec<_>>();
        attributes.sort_unstable_by(|left, right| left.0.cmp(right.0));
        let mut map = serializer.serialize_map(Some(attributes.len() + 2))?;
        map.serialize_entry("source", &self.edge.source)?;
        map.serialize_entry("target", &self.edge.target)?;
        for (key, value) in attributes {
            map.serialize_entry(key, &FactDigestValue(value))?;
        }
        map.end()
    }
}

struct FactDigestEdges<'a> {
    edges: &'a [RawEdgeRecord],
}

impl Serialize for FactDigestEdges<'_> {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        let mut edges = self.edges.iter().collect::<Vec<_>>();
        edges.sort_unstable_by(|left, right| {
            left.source
                .cmp(&right.source)
                .then_with(|| left.target.cmp(&right.target))
                .then_with(|| compare_fact_maps(&left.attributes, &right.attributes))
        });
        let mut sequence = serializer.serialize_seq(Some(self.edges.len()))?;
        for edge in edges {
            sequence.serialize_element(&FactDigestEdge { edge })?;
        }
        sequence.end()
    }
}

struct FactDigestCall<'a> {
    call: &'a RawCall,
}

impl Serialize for FactDigestCall<'_> {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        let call = self.call;
        let mut map = serializer.serialize_map(None)?;
        map.serialize_entry("caller_nid", &call.caller_nid)?;
        map.serialize_entry("callee", &call.callee)?;
        if let Some(value) = call.is_member_call {
            map.serialize_entry("is_member_call", &value)?;
        }
        map.serialize_entry("source_file", &call.source_file)?;
        map.serialize_entry("source_location", &call.source_location)?;
        if let Some(Some(value)) = call.receiver.as_ref() {
            map.serialize_entry("receiver", value)?;
        }
        if let Some(Some(value)) = call.receiver_type.as_ref() {
            map.serialize_entry("receiver_type", value)?;
        }
        if let Some(value) = &call.lang {
            map.serialize_entry("lang", value)?;
        }
        let mut extensions = call.extensions.iter().collect::<Vec<_>>();
        extensions.sort_unstable_by(|left, right| left.0.cmp(right.0));
        for (key, value) in extensions {
            map.serialize_entry(key, &FactDigestValue(value))?;
        }
        map.end()
    }
}

struct FactDigestCalls<'a> {
    calls: &'a [RawCall],
}

impl Serialize for FactDigestCalls<'_> {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        let mut calls = self.calls.iter().collect::<Vec<_>>();
        calls.sort_unstable_by(|left, right| {
            left.caller_nid
                .cmp(&right.caller_nid)
                .then_with(|| left.callee.cmp(&right.callee))
                .then_with(|| left.is_member_call.cmp(&right.is_member_call))
                .then_with(|| left.source_file.cmp(&right.source_file))
                .then_with(|| left.source_location.cmp(&right.source_location))
                .then_with(|| {
                    normalized_call_text(&left.receiver).cmp(&normalized_call_text(&right.receiver))
                })
                .then_with(|| {
                    normalized_call_text(&left.receiver_type)
                        .cmp(&normalized_call_text(&right.receiver_type))
                })
                .then_with(|| left.lang.cmp(&right.lang))
                .then_with(|| compare_fact_maps(&left.extensions, &right.extensions))
        });
        let mut sequence = serializer.serialize_seq(Some(self.calls.len()))?;
        for call in calls {
            sequence.serialize_element(&FactDigestCall { call })?;
        }
        sequence.end()
    }
}

struct FactDigestFrameworkFacts<'a> {
    facts: &'a [RawFrameworkFact],
}

impl Serialize for FactDigestFrameworkFacts<'_> {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        let mut facts = self
            .facts
            .iter()
            .map(|fact| {
                serde_json::to_value(fact)
                    .map(|value| (value, fact))
                    .map_err(S::Error::custom)
            })
            .collect::<Result<Vec<_>, _>>()?;
        facts.sort_unstable_by(|left, right| compare_fact_values(&left.0, &right.0));
        let mut sequence = serializer.serialize_seq(Some(self.facts.len()))?;
        for (value, _) in facts {
            sequence.serialize_element(&FactDigestValue(&value))?;
        }
        sequence.end()
    }
}

struct FactDigestEvidenceContext<'a> {
    declaration_keys: BTreeMap<&'a str, &'a str>,
    file_declarations: BTreeSet<&'a str>,
    scope_keys: BTreeMap<&'a str, String>,
}

impl<'a> FactDigestEvidenceContext<'a> {
    fn new(batch: &'a SemanticEvidenceBatch, file_ids: &'a BTreeSet<&'a str>) -> Self {
        let declaration_keys = batch
            .declarations
            .iter()
            .map(|declaration| (declaration.id.as_str(), declaration.graph_node_id.as_str()))
            .collect::<BTreeMap<_, _>>();
        let file_declarations = batch
            .declarations
            .iter()
            .filter(|declaration| {
                declaration.kind == "file" || file_ids.contains(declaration.graph_node_id.as_str())
            })
            .map(|declaration| declaration.id.as_str())
            .collect::<BTreeSet<_>>();
        let scope_bases = batch
            .scopes
            .iter()
            .map(|scope| {
                let owner = scope
                    .owner_declaration_id
                    .as_deref()
                    .map(|id| {
                        declaration_keys
                            .get(id)
                            .map_or_else(|| format!("raw:{id}"), |key| (*key).to_owned())
                    })
                    .unwrap_or_else(|| "none".to_owned());
                (
                    scope.id.as_str(),
                    format!("scope|{}|{}|owner={owner}", scope.language, scope.kind),
                )
            })
            .collect::<BTreeMap<_, _>>();
        let mut scope_keys = scope_bases.clone();

        // Scope IDs are derived from source ranges. Rebuild them from the
        // stable owner/kind/parent shape so a file-level range change does not
        // invalidate the semantic fact digest. The fixed point is bounded by
        // the number of scopes and falls back to raw parent IDs when a
        // malformed batch references an unknown scope.
        for _ in 0..=batch.scopes.len().min(1_024) {
            let previous = scope_keys.clone();
            let mut changed = false;
            for scope in &batch.scopes {
                let parent = scope.parent_scope_id.as_deref().map_or_else(
                    || "none".to_owned(),
                    |id| {
                        previous
                            .get(id)
                            .cloned()
                            .unwrap_or_else(|| format!("raw:{id}"))
                    },
                );
                let Some(base) = scope_bases.get(scope.id.as_str()) else {
                    continue;
                };
                let next = format!("{base}|parent={parent}");
                if previous.get(scope.id.as_str()) != Some(&next) {
                    changed = true;
                }
                scope_keys.insert(scope.id.as_str(), next);
            }
            if !changed {
                break;
            }
        }

        Self {
            declaration_keys,
            file_declarations,
            scope_keys,
        }
    }

    fn declaration_key(&self, id: &str) -> Cow<'_, str> {
        self.declaration_keys.get(id).map_or_else(
            || Cow::Owned(format!("raw:{id}")),
            |key| Cow::Borrowed(*key),
        )
    }

    fn declaration_is_file(&self, id: &str) -> bool {
        self.file_declarations.contains(id)
    }

    fn scope_key(&self, id: &str) -> Cow<'_, str> {
        self.scope_keys.get(id).map_or_else(
            || Cow::Owned(format!("raw:{id}")),
            |key| Cow::Borrowed(key.as_str()),
        )
    }
}

struct FactDigestDeclaration<'a, 'context> {
    declaration: &'a DeclarationFact,
    context: &'context FactDigestEvidenceContext<'a>,
}

impl Serialize for FactDigestDeclaration<'_, '_> {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        let declaration = self.declaration;
        let mut map = serializer.serialize_map(None)?;
        map.serialize_entry("id", &self.context.declaration_key(&declaration.id))?;
        map.serialize_entry("language", &declaration.language)?;
        map.serialize_entry("graphNodeId", &declaration.graph_node_id)?;
        map.serialize_entry("kind", &declaration.kind)?;
        map.serialize_entry("name", &declaration.name)?;
        map.serialize_entry("qualifiedName", &declaration.qualified_name)?;
        if let Some(value) = &declaration.module_or_package {
            map.serialize_entry("moduleOrPackage", value)?;
        }
        if let Some(value) = &declaration.scope_id {
            map.serialize_entry("scopeId", &self.context.scope_key(value))?;
        }
        if let Some(value) = &declaration.signature {
            map.serialize_entry("signature", value)?;
        }
        if let Some(value) = declaration.parameter_count {
            map.serialize_entry("parameterCount", &value)?;
        }
        if !declaration.parameter_types.is_empty() {
            map.serialize_entry("parameterTypes", &declaration.parameter_types)?;
        }
        if declaration.direct_bases_complete {
            map.serialize_entry("directBasesComplete", &true)?;
        }
        map.serialize_entry("variadic", &declaration.variadic)?;
        if let Some(value) = &declaration.signature_hash {
            map.serialize_entry("signatureHash", value)?;
        }
        if let Some(value) = &declaration.implementation_hash {
            map.serialize_entry("implementationHash", value)?;
        }
        if let Some(value) = &declaration.source_hash {
            map.serialize_entry("sourceHash", value)?;
        }
        if !self.context.declaration_is_file(&declaration.id) {
            map.serialize_entry("range", &declaration.range)?;
        }
        map.end()
    }
}

struct FactDigestDeclarations<'a, 'context> {
    declarations: &'a [DeclarationFact],
    context: &'context FactDigestEvidenceContext<'a>,
}

impl Serialize for FactDigestDeclarations<'_, '_> {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        let mut declarations = self.declarations.iter().collect::<Vec<_>>();
        declarations.sort_by(|left, right| {
            self.context
                .declaration_key(&left.id)
                .cmp(&self.context.declaration_key(&right.id))
                .then_with(|| left.kind.cmp(&right.kind))
                .then_with(|| left.name.cmp(&right.name))
                .then_with(|| left.range.start_byte.cmp(&right.range.start_byte))
                .then_with(|| left.range.end_byte.cmp(&right.range.end_byte))
        });
        let mut sequence = serializer.serialize_seq(Some(declarations.len()))?;
        for declaration in declarations {
            sequence.serialize_element(&FactDigestDeclaration {
                declaration,
                context: self.context,
            })?;
        }
        sequence.end()
    }
}

struct FactDigestScope<'a, 'context> {
    scope: &'a ScopeFact,
    context: &'context FactDigestEvidenceContext<'a>,
}

impl Serialize for FactDigestScope<'_, '_> {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        let mut map = serializer.serialize_map(None)?;
        map.serialize_entry("id", &self.context.scope_key(&self.scope.id))?;
        map.serialize_entry("language", &self.scope.language)?;
        map.serialize_entry("kind", &self.scope.kind)?;
        if let Some(owner) = &self.scope.owner_declaration_id {
            map.serialize_entry("ownerDeclarationId", &self.context.declaration_key(owner))?;
        }
        if let Some(parent) = &self.scope.parent_scope_id {
            map.serialize_entry("parentScopeId", &self.context.scope_key(parent))?;
        }
        let file_scope = self
            .scope
            .owner_declaration_id
            .as_deref()
            .is_some_and(|owner| self.context.declaration_is_file(owner));
        if !file_scope {
            map.serialize_entry("range", &self.scope.range)?;
        }
        map.end()
    }
}

struct FactDigestBinding<'a, 'context> {
    binding: &'a BindingFact,
    context: &'context FactDigestEvidenceContext<'a>,
}

impl Serialize for FactDigestBinding<'_, '_> {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        let binding = self.binding;
        let mut map = serializer.serialize_map(None)?;
        map.serialize_entry("language", &binding.language)?;
        map.serialize_entry("kind", &binding.kind)?;
        map.serialize_entry("spelling", &binding.spelling)?;
        map.serialize_entry("qualifiedTarget", &binding.qualified_target)?;
        if let Some(target) = &binding.target_declaration_id {
            map.serialize_entry("targetDeclarationId", &self.context.declaration_key(target))?;
        }
        if let Some(scope) = &binding.scope_id {
            map.serialize_entry("scopeId", &self.context.scope_key(scope))?;
        }
        if let Some(output_index) = binding.output_index {
            map.serialize_entry("outputIndex", &output_index)?;
        }
        map.serialize_entry("range", &binding.range)?;
        map.end()
    }
}

struct FactDigestOccurrence<'a, 'context> {
    occurrence: &'a OccurrenceFact,
    context: &'context FactDigestEvidenceContext<'a>,
}

impl Serialize for FactDigestOccurrence<'_, '_> {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        let occurrence = self.occurrence;
        let mut map = serializer.serialize_map(None)?;
        map.serialize_entry("language", &occurrence.language)?;
        map.serialize_entry("role", &occurrence.role)?;
        map.serialize_entry(
            "ownerDeclarationId",
            &self
                .context
                .declaration_key(&occurrence.owner_declaration_id),
        )?;
        map.serialize_entry("spelling", &occurrence.spelling)?;
        if let Some(value) = &occurrence.qualifier {
            map.serialize_entry("qualifier", value)?;
        }
        if let Some(value) = &occurrence.context {
            map.serialize_entry("context", value)?;
        }
        if let Some(scope) = &occurrence.scope_id {
            map.serialize_entry("scopeId", &self.context.scope_key(scope))?;
        }
        map.serialize_entry("range", &occurrence.range)?;
        map.end()
    }
}

struct FactDigestResolutionConstraint<'a, 'context> {
    constraint: &'a ResolutionConstraint,
    context: &'context FactDigestEvidenceContext<'a>,
}

impl Serialize for FactDigestResolutionConstraint<'_, '_> {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        let constraint = self.constraint;
        let mut map = serializer.serialize_map(None)?;
        if let Some(value) = &constraint.exact_target_declaration_id {
            map.serialize_entry(
                "exactTargetDeclarationId",
                &self.context.declaration_key(value),
            )?;
        }
        if let Some(value) = &constraint.exact_language {
            map.serialize_entry("exactLanguage", value)?;
        }
        if let Some(value) = &constraint.module_or_package {
            map.serialize_entry("moduleOrPackage", value)?;
        }
        if let Some(value) = &constraint.scope_id {
            map.serialize_entry("scopeId", &self.context.scope_key(value))?;
        }
        if let Some(value) = &constraint.qualified_name {
            map.serialize_entry("qualifiedName", value)?;
        }
        if let Some(value) = constraint.argument_count {
            map.serialize_entry("argumentCount", &value)?;
        }
        if !constraint.argument_types.is_empty() {
            map.serialize_entry("argumentTypes", &constraint.argument_types)?;
        }
        if !constraint.allowed_target_kinds.is_empty() {
            map.serialize_entry("allowedTargetKinds", &constraint.allowed_target_kinds)?;
        }
        if let Some(value) = &constraint.hierarchy {
            map.serialize_entry("hierarchy", value)?;
        }
        map.serialize_entry("allowExternal", &constraint.allow_external)?;
        map.end()
    }
}

struct FactDigestCandidate<'a, 'context> {
    candidate: &'a RelationshipCandidate,
    context: &'context FactDigestEvidenceContext<'a>,
}

impl Serialize for FactDigestCandidate<'_, '_> {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        let candidate = self.candidate;
        let mut map = serializer.serialize_map(None)?;
        map.serialize_entry("language", &candidate.language)?;
        map.serialize_entry("relation", &candidate.relation)?;
        map.serialize_entry(
            "sourceDeclarationId",
            &self
                .context
                .declaration_key(&candidate.source_declaration_id),
        )?;
        map.serialize_entry("targetSpelling", &candidate.target_spelling)?;
        map.serialize_entry(
            "constraints",
            &FactDigestResolutionConstraint {
                constraint: &candidate.constraints,
                context: self.context,
            },
        )?;
        map.end()
    }
}

struct FactDigestSemanticEvidence<'a> {
    batch: &'a SemanticEvidenceBatch,
    file_ids: &'a BTreeSet<&'a str>,
}

impl Serialize for FactDigestSemanticEvidence<'_> {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        let context = FactDigestEvidenceContext::new(self.batch, self.file_ids);
        let mut scopes = self.batch.scopes.iter().collect::<Vec<_>>();
        scopes.sort_by(|left, right| {
            context
                .scope_key(&left.id)
                .cmp(&context.scope_key(&right.id))
                .then_with(|| left.range.start_byte.cmp(&right.range.start_byte))
                .then_with(|| left.range.end_byte.cmp(&right.range.end_byte))
        });
        let mut bindings = self.batch.bindings.iter().collect::<Vec<_>>();
        bindings.sort_by(|left, right| {
            left.range
                .source_file
                .cmp(&right.range.source_file)
                .then_with(|| left.range.start_byte.cmp(&right.range.start_byte))
                .then_with(|| left.range.end_byte.cmp(&right.range.end_byte))
                .then_with(|| left.spelling.cmp(&right.spelling))
                .then_with(|| left.qualified_target.cmp(&right.qualified_target))
                .then_with(|| left.kind.cmp(&right.kind))
        });
        let mut occurrences = self.batch.occurrences.iter().collect::<Vec<_>>();
        occurrences.sort_by(|left, right| {
            left.range
                .source_file
                .cmp(&right.range.source_file)
                .then_with(|| left.range.start_byte.cmp(&right.range.start_byte))
                .then_with(|| left.range.end_byte.cmp(&right.range.end_byte))
                .then_with(|| left.spelling.cmp(&right.spelling))
                .then_with(|| left.role.cmp(&right.role))
        });
        let mut candidates = self.batch.candidates.iter().collect::<Vec<_>>();
        candidates.sort_by(|left, right| {
            context
                .declaration_key(&left.source_declaration_id)
                .cmp(&context.declaration_key(&right.source_declaration_id))
                .then_with(|| left.relation.cmp(&right.relation))
                .then_with(|| left.target_spelling.cmp(&right.target_spelling))
                .then_with(|| {
                    left.constraints
                        .qualified_name
                        .cmp(&right.constraints.qualified_name)
                })
                .then_with(|| {
                    let left_target = left
                        .constraints
                        .exact_target_declaration_id
                        .as_deref()
                        .map_or(Cow::Borrowed(""), |id| context.declaration_key(id));
                    let right_target = right
                        .constraints
                        .exact_target_declaration_id
                        .as_deref()
                        .map_or(Cow::Borrowed(""), |id| context.declaration_key(id));
                    left_target.cmp(&right_target)
                })
                .then_with(|| {
                    let left_scope = left
                        .constraints
                        .scope_id
                        .as_deref()
                        .map_or(Cow::Borrowed(""), |id| context.scope_key(id));
                    let right_scope = right
                        .constraints
                        .scope_id
                        .as_deref()
                        .map_or(Cow::Borrowed(""), |id| context.scope_key(id));
                    left_scope.cmp(&right_scope)
                })
                .then_with(|| {
                    left.constraints
                        .argument_count
                        .cmp(&right.constraints.argument_count)
                })
                .then_with(|| {
                    left.constraints
                        .allowed_target_kinds
                        .cmp(&right.constraints.allowed_target_kinds)
                })
        });
        let mut map = serializer.serialize_map(Some(7))?;
        map.serialize_entry("adapter", &self.batch.adapter)?;
        map.serialize_entry(
            "declarations",
            &FactDigestDeclarations {
                declarations: &self.batch.declarations,
                context: &context,
            },
        )?;
        map.serialize_entry(
            "scopes",
            &scopes
                .iter()
                .map(|scope| FactDigestScope {
                    scope,
                    context: &context,
                })
                .collect::<Vec<_>>(),
        )?;
        map.serialize_entry(
            "bindings",
            &bindings
                .iter()
                .map(|binding| FactDigestBinding {
                    binding,
                    context: &context,
                })
                .collect::<Vec<_>>(),
        )?;
        map.serialize_entry(
            "occurrences",
            &occurrences
                .iter()
                .map(|occurrence| FactDigestOccurrence {
                    occurrence,
                    context: &context,
                })
                .collect::<Vec<_>>(),
        )?;
        map.serialize_entry(
            "candidates",
            &candidates
                .iter()
                .map(|candidate| FactDigestCandidate {
                    candidate,
                    context: &context,
                })
                .collect::<Vec<_>>(),
        )?;
        if !self.batch.diagnostics.is_empty() {
            map.serialize_entry("diagnostics", &self.batch.diagnostics)?;
        }
        map.end()
    }
}

struct FactDigestExtraction<'a> {
    extraction: &'a Extraction,
}

impl Serialize for FactDigestExtraction<'_> {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        let file_ids = self
            .extraction
            .nodes
            .iter()
            .filter(|node| node.string("symbol_kind") == "file")
            .map(|node| node.id.as_str())
            .collect::<BTreeSet<_>>();
        let mut map = serializer.serialize_map(None)?;
        map.serialize_entry(
            "nodes",
            &FactDigestNodes {
                nodes: &self.extraction.nodes,
            },
        )?;
        map.serialize_entry(
            "edges",
            &FactDigestEdges {
                edges: &self.extraction.edges,
            },
        )?;
        if !self.extraction.hyperedges.is_empty() {
            let hyperedges = self
                .extraction
                .hyperedges
                .iter()
                .map(FactDigestValue)
                .collect::<Vec<_>>();
            map.serialize_entry("hyperedges", &hyperedges)?;
        }
        if let Some(raw_calls) = &self.extraction.raw_calls {
            map.serialize_entry("raw_calls", &FactDigestCalls { calls: raw_calls })?;
        }
        if !self.extraction.framework_facts.is_empty() {
            map.serialize_entry(
                "framework_facts",
                &FactDigestFrameworkFacts {
                    facts: &self.extraction.framework_facts,
                },
            )?;
        }
        if let Some(batch) = &self.extraction.semantic_evidence {
            map.serialize_entry(
                "semantic_evidence",
                &FactDigestSemanticEvidence {
                    batch,
                    file_ids: &file_ids,
                },
            )?;
        }
        if let Some(error) = &self.extraction.error {
            map.serialize_entry("error", error)?;
        }
        let mut extensions = self.extraction.extensions.iter().collect::<Vec<_>>();
        extensions.sort_unstable_by(|left, right| left.0.cmp(right.0));
        for (key, value) in extensions {
            map.serialize_entry(key, &FactDigestValue(value))?;
        }
        map.end()
    }
}

fn extraction_fact_digest(extraction: &Extraction) -> Result<String, serde_json::Error> {
    let mut writer = ExtractionDigestWriter(Sha256::new());
    {
        let mut serializer = serde_json::Serializer::new(&mut writer);
        FactDigestExtraction { extraction }.serialize(&mut serializer)?;
    }
    Ok(format!("sha256:{:x}", writer.0.finalize()))
}

fn ast_fact_digest_entries(
    paths: &[PathBuf],
    extractions: &[Extraction],
    output_dir: &Path,
    root: &Path,
) -> Result<BTreeMap<String, String>, CoreError> {
    let digest_entry = |(path, extraction): (&PathBuf, &Extraction)| {
        let digest = extraction_fact_digest(extraction).map_err(|source| {
            CoreError::SerializeExtraction {
                path: output_dir.join(AST_FACT_DIGESTS_FILE),
                source,
            }
        })?;
        Ok((relative_fact_path(path, root), digest))
    };
    let entries = if !should_parallel_ast_fact_digest(paths.len()) {
        paths
            .iter()
            .zip(extractions)
            .map(digest_entry)
            .collect::<Vec<_>>()
    } else {
        paths
            .par_iter()
            .zip(extractions.par_iter())
            .map(digest_entry)
            .collect::<Vec<_>>()
    };
    // Indexed parallel collection preserves source order. Resolve errors and
    // build the deterministic contract map sequentially so a malformed fact
    // set reports the same first failure regardless of worker scheduling.
    entries.into_iter().collect()
}

const fn should_parallel_ast_fact_digest(files: usize) -> bool {
    files >= PARALLEL_AST_FACT_DIGEST_MIN_FILES
}

fn detected_file_sets_match(manifest: &Manifest, files: &BTreeMap<String, Vec<String>>) -> bool {
    let current = files
        .values()
        .flatten()
        .map(PathBuf::from)
        .map(|path| canonical_identity(&path))
        .collect::<BTreeSet<_>>();
    let previous = manifest
        .entries()
        .keys()
        .map(PathBuf::from)
        .map(|path| canonical_identity(&path))
        .collect::<BTreeSet<_>>();
    current == previous
}

fn source_removed_from_detection(
    manifest: &Manifest,
    detected_files: &BTreeMap<String, Vec<String>>,
) -> bool {
    let live_sources = detected_files
        .values()
        .flatten()
        .map(|path| canonical_identity(Path::new(path)))
        .collect::<HashSet<_>>();
    manifest
        .entries()
        .keys()
        .map(|path| canonical_identity(Path::new(path)))
        .any(|path| !live_sources.contains(&path))
}

#[allow(clippy::too_many_arguments)]
fn fact_neutral_pre_cache_sources(
    options: &BuildOptions,
    semantic: Option<&SemanticLayer>,
    supplemental: &[Extraction],
    retain_artifacts: bool,
    prior_state: Option<&AstFactDigestState>,
    profile_digest: &str,
    project_evidence_digest: &str,
    manifest: &Manifest,
    detected_files: &BTreeMap<String, Vec<String>>,
    sources: &[PathBuf],
    preserve_prior_semantic: bool,
    root: &Path,
) -> Option<Vec<PathBuf>> {
    let has_nonempty_semantic = semantic.is_some_and(|layer| !semantic_layer_is_empty(layer));
    if options.force
        || options.purpose != BuildPurpose::Extract
        || options.program_analysis
        || has_nonempty_semantic
        || !supplemental.is_empty()
        || retain_artifacts
        || preserve_prior_semantic
        || !detected_file_sets_match(manifest, detected_files)
        || source_removed_from_detection(manifest, detected_files)
    {
        return None;
    }
    let prior_state = prior_state?;
    let expected_paths = sources
        .iter()
        .map(|path| relative_fact_path(path, root))
        .collect::<BTreeSet<_>>();
    if prior_state.profile_digest != profile_digest
        || prior_state.project_evidence_digest != project_evidence_digest
        || prior_state.entries.len() != expected_paths.len()
        || prior_state.entries.keys().ne(expected_paths.iter())
    {
        return None;
    }
    let missing = sources
        .iter()
        .filter(|path| manifest.is_changed(path, ManifestKind::Ast))
        .cloned()
        .collect::<Vec<_>>();
    (!missing.is_empty()).then_some(missing)
}

fn extract_fact_neutral_sources(
    paths: &[PathBuf],
    options: &BuildOptions,
    root: &Path,
    project_evidence: &Arc<ProjectEvidenceIndex>,
) -> Result<Option<FactNeutralExtractionBatch>, CoreError> {
    let mut engine = Engine::with_project_evidence(Arc::clone(project_evidence));
    let mut extractions = Vec::with_capacity(paths.len());
    let mut source_digests = BTreeMap::new();
    let mut empty_files = Vec::new();
    let mut extraction_failures = BTreeMap::new();
    let mut extraction_partials = BTreeMap::new();
    for path in paths {
        let metadata = fs::metadata(path).map_err(|source| compass_files::FileError::Io {
            path: path.clone(),
            source,
        })?;
        if !metadata.is_file() || metadata.len() > options.max_source_bytes {
            return Ok(None);
        }
        let bytes = fs::read(path).map_err(|source| compass_files::FileError::Io {
            path: path.clone(),
            source,
        })?;
        let source_file = path
            .strip_prefix(root)
            .unwrap_or(path)
            .to_string_lossy()
            .replace('\\', "/");
        let mut extraction = match engine.extract_source_graph_only(path, &source_file, &bytes) {
            Ok(extraction) => extraction,
            Err(_) => return Ok(None),
        };
        prepare_extraction_for_publication(
            path,
            &mut extraction,
            root,
            &mut extraction_failures,
            &mut extraction_partials,
        );
        if !extraction_failures.is_empty() || !extraction_partials.is_empty() {
            return Ok(None);
        }
        if !extraction_has_cacheable_ast_facts(&extraction) {
            empty_files.push(path.clone());
        }
        source_digests.insert(
            relative_fact_path(path, root),
            SourceDigest {
                content_digest: format!("sha256:{:x}", Sha256::digest(&bytes)),
                byte_size: bytes.len() as u64,
            },
        );
        prepare_portable_ast_cache_entry(&mut extraction, path, root);
        extractions.push(extraction);
    }
    // Keep the digest representation aligned with the normal cold path. The
    // changed-file set is sufficient for the neutral proof because every
    // unchanged extraction is covered by the prior sidecar's exact digest.
    merge_decl_def_classes_if_needed(&mut extractions, paths);
    extractions.iter_mut().for_each(compact_extraction);
    if extractions.is_empty() {
        return Ok(None);
    }
    Ok(Some(FactNeutralExtractionBatch {
        extractions,
        source_digests,
        empty_files,
    }))
}

#[allow(clippy::too_many_arguments)]
fn fact_neutral_incremental_candidate(
    options: &BuildOptions,
    semantic: Option<&SemanticLayer>,
    supplemental: &[Extraction],
    retain_artifacts: bool,
    prior_state: Option<&AstFactDigestState>,
    current_state: &AstFactDigestState,
    manifest: &Manifest,
    detected_files: &BTreeMap<String, Vec<String>>,
    missing: &[PathBuf],
    source_removed: bool,
    root: &Path,
) -> bool {
    let has_nonempty_semantic = semantic.is_some_and(|layer| !semantic_layer_is_empty(layer));
    if options.force
        || options.purpose != BuildPurpose::Extract
        || options.program_analysis
        || has_nonempty_semantic
        || !supplemental.is_empty()
        || retain_artifacts
        || missing.is_empty()
        || source_removed
        || !detected_file_sets_match(manifest, detected_files)
    {
        return false;
    }
    let Some(prior_state) = prior_state else {
        return false;
    };
    if prior_state.profile_digest != current_state.profile_digest
        || prior_state.project_evidence_digest != current_state.project_evidence_digest
        || prior_state.entries.len() != current_state.entries.len()
        || prior_state.entries.keys().ne(current_state.entries.keys())
    {
        return false;
    }
    missing.iter().all(|path| {
        let key = relative_fact_path(path, root);
        prior_state.entries.get(&key) == current_state.entries.get(&key)
    })
}

fn extraction_file_anchors(
    extractions: &[Extraction],
    root: &Path,
) -> BTreeMap<String, SourceAnchor> {
    extractions
        .iter()
        .flat_map(|extraction| &extraction.nodes)
        .filter(|node| node.string("symbol_kind") == "file")
        .filter_map(|node| {
            let source = node.string("source_file");
            let anchor = node
                .attributes
                .get("source_anchor")
                .cloned()
                .and_then(|value| serde_json::from_value::<SourceAnchor>(value).ok())?;
            Some((relative_fact_path(Path::new(&source), root), anchor))
        })
        .collect()
}

fn full_file_source_anchor(
    path: &Path,
    relative_path: &str,
    max_source_bytes: u64,
) -> Option<SourceAnchor> {
    let metadata = fs::metadata(path).ok()?;
    if !metadata.is_file() || metadata.len() > max_source_bytes {
        return None;
    }
    let bytes = fs::read(path).ok()?;
    let end_line_index = bytes.iter().filter(|byte| **byte == b'\n').count();
    let end_line_start = bytes
        .iter()
        .rposition(|byte| *byte == b'\n')
        .map_or(0, |index| index.saturating_add(1));
    Some(SourceAnchor {
        file: relative_path.to_owned(),
        start_byte: 0,
        end_byte: bytes.len() as u64,
        start_line: 1,
        start_column: 0,
        end_line: u32::try_from(end_line_index.saturating_add(1)).unwrap_or(u32::MAX),
        end_column: u32::try_from(bytes.len().saturating_sub(end_line_start)).unwrap_or(u32::MAX),
    })
}

type FactNeutralDocument = (Option<V1GraphDocument>, V1GraphDocument, BTreeSet<String>);

fn sort_dedup_serialized<T: Serialize>(values: &mut Vec<T>) {
    if values.len() < 2 {
        return;
    }
    let mut keyed = values
        .drain(..)
        .map(|value| (serde_json::to_vec(&value).unwrap_or_default(), value))
        .collect::<Vec<_>>();
    keyed.sort_unstable_by(|left, right| left.0.cmp(&right.0));
    let mut previous = None;
    for (key, value) in keyed {
        if previous.as_ref() == Some(&key) {
            continue;
        }
        previous = Some(key);
        values.push(value);
    }
}

fn canonicalize_fact_neutral_metadata(
    coverage: &mut Vec<CoverageRecord>,
    diagnostics: &mut Vec<GraphDiagnostic>,
) {
    // Fact-neutral publication bypasses the normal v1 normalization pass, so
    // preserve the same metadata ordering and duplicate policy here. Without
    // this, appending the refreshed file inventory to the prior graph changes
    // coverage order even when the semantic graph is unchanged.
    sort_dedup_serialized(coverage);
    coverage.sort_by(|left, right| {
        (
            left.capability.as_str(),
            left.producer.as_str(),
            left.file_id.as_deref(),
        )
            .cmp(&(
                right.capability.as_str(),
                right.producer.as_str(),
                right.file_id.as_deref(),
            ))
    });
    sort_dedup_serialized(diagnostics);
    diagnostics.sort_by(|left, right| {
        (left.code.as_str(), left.message.as_str())
            .cmp(&(right.code.as_str(), right.message.as_str()))
    });
}

#[allow(clippy::too_many_arguments)]
fn prepare_fact_neutral_document(
    output_dir: &Path,
    root: &Path,
    source_digests: &BTreeMap<String, SourceDigest>,
    inventory: Vec<InventoryEvidence>,
    extractions: &[Extraction],
    configuration_digest: String,
    max_source_bytes: u64,
    retain_previous: bool,
) -> Result<Option<FactNeutralDocument>, CoreError> {
    let graph_path = output_dir.join("graph.json");
    let Ok(previous) = V1GraphDocument::load(&graph_path) else {
        return Ok(None);
    };
    // JSON publication only needs the updated document. Keeping a second full
    // graph alive here doubled the largest resident allocation on neutral
    // edits. SQLite publication still retains the previous document because
    // the object store needs it to compute the file-node delta.
    let (previous_for_delta, mut current) = if retain_previous {
        (Some(previous.clone()), previous)
    } else {
        (None, previous)
    };
    let mut changed_file_node_ids = BTreeSet::new();
    let mut evidence = BuildEvidence::new(root.to_path_buf(), current.graph.build.clone());
    evidence.files = current.graph.files.clone();
    evidence.coverage = current
        .graph
        .coverage
        .iter()
        .filter(|coverage| coverage.capability != "file_inventory")
        .cloned()
        .collect();
    evidence.diagnostics = current
        .graph
        .diagnostics
        .iter()
        .filter(|diagnostic| {
            !matches!(
                diagnostic.code.as_str(),
                "parser_recovery"
                    | "partial_extraction"
                    | "extractor_failure"
                    | "unsupported_input"
                    | "excluded_input"
                    | "generated_input"
                    | "binary_input"
            )
        })
        .cloned()
        .collect();
    for file in &mut evidence.files {
        if let Some(digest) = source_digests.get(&file.path) {
            file.content_digest.clone_from(&digest.content_digest);
            file.byte_size = digest.byte_size;
        }
    }
    evidence.build.configuration_digest = configuration_digest;
    evidence.include_inventory(inventory)?;
    canonicalize_fact_neutral_metadata(&mut evidence.coverage, &mut evidence.diagnostics);

    let anchors = extraction_file_anchors(extractions, root);
    current.graph.build = evidence.build;
    current.graph.files = evidence.files;
    current.graph.coverage = evidence.coverage;
    current.graph.diagnostics = evidence.diagnostics;
    let files = current
        .graph
        .files
        .iter()
        .map(|file| (file.path.as_str(), file))
        .collect::<BTreeMap<_, _>>();
    for node in &mut current.nodes {
        if node.kind != NodeKind::File {
            continue;
        }
        let before = node.clone();
        let Some(source) = node.source_file() else {
            continue;
        };
        let source = relative_fact_path(Path::new(source), root);
        let Some(file) = files.get(source.as_str()) else {
            continue;
        };
        node.details = Some(NodeDetails::File(FileNodeDetails {
            content_digest: file.content_digest.clone(),
            byte_size: file.byte_size,
            generated: file.generated,
        }));
        let anchor = source_digests.contains_key(&source).then(|| {
            anchors
                .get(&source)
                .cloned()
                .or_else(|| full_file_source_anchor(&root.join(&source), &source, max_source_bytes))
        });
        if let Some(anchor) = anchor.flatten() {
            node.source = Some(anchor.clone());
            for provenance in &mut node.evidence {
                for candidate in &mut provenance.anchors {
                    if candidate.file == source {
                        candidate.clone_from(&anchor);
                    }
                }
                if provenance
                    .wiring_site
                    .as_ref()
                    .is_some_and(|candidate| candidate.file == source)
                {
                    provenance.wiring_site = Some(anchor.clone());
                }
            }
        }
        if *node != before {
            changed_file_node_ids.insert(node.id.clone());
        }
    }
    compass_model::validate_code_graph(&current).map_err(|error| {
        CoreError::InvalidBuildState(format!(
            "fact-neutral incremental graph failed validation: {error}"
        ))
    })?;
    Ok(Some((previous_for_delta, current, changed_file_node_ids)))
}

/// Keep the byte-preserving JSON delta bounded independently from the graph
/// reader cap. Large graphs fall back to the existing streaming serializer so
/// an incremental edit never adds another large resident byte buffer.
fn read_fact_neutral_delta_source(path: &Path) -> Option<Vec<u8>> {
    let metadata = fs::metadata(path).ok()?;
    if !metadata.is_file() || metadata.len() > GRAPH_JSON_DELTA_MAX_SOURCE_BYTES as u64 {
        return None;
    }
    fs::read(path).ok()
}

#[allow(clippy::too_many_arguments)]
fn publish_fact_neutral_incremental(
    options: &BuildOptions,
    root: PathBuf,
    output_dir: PathBuf,
    output_container: &Path,
    guard: BuildGuard,
    manifest_path: &Path,
    mut prior_manifest: Manifest,
    detection: Detection,
    sources: &[PathBuf],
    missing: &[PathBuf],
    empty_files: Vec<PathBuf>,
    previous: Option<&V1GraphDocument>,
    current: &V1GraphDocument,
    changed_file_node_ids: &BTreeSet<String>,
    fact_state: &AstFactDigestState,
    timings: &mut BuildTimings,
) -> Result<BuildResult, CoreError> {
    let publish_started = Instant::now();
    let published_nodes = current.nodes.len();
    let published_edges = current.links.len();
    let omissions = saved_publication_omissions(&output_dir);
    let graph_path = output_dir.join("graph.json");
    let (store_metrics, graph_seal) = if options.graph_storage.publishes_store() {
        let previous = previous.ok_or_else(|| {
            CoreError::InvalidBuildState(
                "fact-neutral SQLite publication is missing its prior graph".to_owned(),
            )
        })?;
        let (metrics, seal) = publish_graph_and_store_delta(&output_dir, previous, current)?;
        (Some(metrics), Some(seal))
    } else {
        let previous_bytes = read_fact_neutral_delta_source(&graph_path);
        let receipt = write_atomic_with_digest(&graph_path, |writer| {
            let delta_started = Instant::now();
            let used_delta = if let Some(bytes) = previous_bytes.as_deref() {
                write_fact_neutral_graph_json_delta_prevalidated(
                    bytes,
                    current,
                    changed_file_node_ids,
                    writer,
                )
                .map_err(|source| compass_files::FileError::Io {
                    path: graph_path.clone(),
                    source,
                })?
            } else {
                false
            };
            profile_internal_duration(
                if used_delta {
                    "fact-neutral graph JSON delta"
                } else {
                    "fact-neutral graph JSON delta fallback"
                },
                delta_started.elapsed(),
            );
            if used_delta {
                Ok(())
            } else {
                write_canonical_graph_json(current, writer).map_err(|source| {
                    compass_files::FileError::Io {
                        path: graph_path.clone(),
                        source,
                    }
                })
            }
        })?;
        (
            None,
            Some(ArtifactSeal {
                bytes: receipt.bytes,
                sha256: receipt.sha256,
            }),
        )
    };
    if let Some(metrics) = store_metrics {
        record_store_metrics(timings, metrics);
    }
    let community_ids = current
        .nodes
        .iter()
        .filter_map(|node| node.community.as_ref().map(|community| community.id))
        .collect::<BTreeSet<_>>();
    let communities = community_ids.len();
    let clustered = !options.no_cluster;
    if !clustered {
        remove_if_exists(&output_dir.join(GRAPH_OVERVIEW_FILE))?;
    }
    save_output_stats(
        &output_dir,
        published_nodes,
        published_edges,
        communities,
        clustered,
        omissions,
    )?;
    write_ast_fact_digest_state(&output_dir, fact_state)?;
    write_semantic_marker(&output_dir, None)?;
    remove_if_exists(&output_dir.join("needs_update"))?;
    save_build_manifest(
        &mut prior_manifest,
        &detection.files,
        manifest_path,
        &root,
        None,
    )?;
    publish_build_state(
        options,
        &output_dir,
        manifest_path,
        sources.len(),
        published_nodes,
        published_edges,
        communities,
        omissions,
        None,
        graph_seal,
        store_metrics.is_some(),
        timings,
    )?;
    let published_output_dir = commit_snapshot(
        guard,
        output_container,
        options.graph_storage,
        store_metrics.is_some(),
        !options.graph_storage.publishes_store(),
        true,
        timings,
    )?;
    timings.publish = publish_started.elapsed();
    Ok(BuildResult {
        root,
        output_dir: published_output_dir,
        detection,
        files_considered: sources.len(),
        files_extracted: missing.len(),
        files_cached: sources.len().saturating_sub(missing.len()),
        empty_files,
        nodes: published_nodes,
        edges: published_edges,
        communities,
        omitted_nodes: omissions.nodes,
        omitted_edges: omissions.edges,
        identity_collisions: omissions.identity_collisions,
        partial_graph: omissions.is_partial(),
        html_written: false,
        outputs_changed: true,
        program_modules: 0,
        program_summaries: 0,
        program_syntax_analyzed: 0,
        program_syntax_reused: 0,
        program_artifacts_loaded: 0,
        program_artifacts_reused: 0,
        program_artifact_documents_analyzed: 0,
        program_artifact_documents_reused: 0,
        program_conflicts: 0,
        timings: *timings,
    })
}

#[derive(Clone, Debug)]
pub struct BuildResult {
    pub root: PathBuf,
    pub output_dir: PathBuf,
    pub detection: Detection,
    pub files_considered: usize,
    pub files_extracted: usize,
    pub files_cached: usize,
    pub empty_files: Vec<PathBuf>,
    pub nodes: usize,
    pub edges: usize,
    pub communities: usize,
    pub omitted_nodes: usize,
    pub omitted_edges: usize,
    pub identity_collisions: usize,
    pub partial_graph: bool,
    pub html_written: bool,
    pub outputs_changed: bool,
    pub program_modules: usize,
    pub program_summaries: usize,
    pub program_syntax_analyzed: usize,
    pub program_syntax_reused: usize,
    pub program_artifacts_loaded: usize,
    pub program_artifacts_reused: usize,
    pub program_artifact_documents_analyzed: usize,
    pub program_artifact_documents_reused: usize,
    pub program_conflicts: usize,
    pub timings: BuildTimings,
}

/// Typed authoritative output retained by an in-process history build.
#[derive(Clone, Debug)]
pub struct RetainedBuildArtifacts {
    pub document: V1GraphDocument,
    pub program: Option<compass_analysis::AnalysisBundle>,
    pub analysis: Option<Value>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct BuildFileProgress {
    pub current: usize,
    pub total: usize,
    pub path: PathBuf,
}

#[derive(Clone, Copy, Debug, Default)]
pub struct BuildTimings {
    pub detect: Duration,
    pub deterministic_extract: Duration,
    pub graph_assembly: Duration,
    pub program_analysis: Duration,
    pub publish: Duration,
    pub store_new_objects: u64,
    pub store_reused_objects: u64,
    pub store_write_transactions: u64,
    pub store_bytes_written: u64,
    pub store_gc_deleted_entries: u64,
}

#[derive(Clone, Copy, Debug, Default)]
struct StorePublishMetrics {
    new_objects: u64,
    reused_objects: u64,
    write_transactions: u64,
    bytes_written: u64,
}

/// Validated semantic output to merge into one atomic graph build.
///
/// `refreshed_files` is the exact set dispatched for this run. Existing
/// semantic facts owned by those sources are removed before the replacement
/// fragment is appended. Partial or uncovered files remain unstamped so the
/// next incremental run retries them.
#[derive(Clone, Debug)]
pub struct SemanticLayer {
    pub fragment: serde_json::Value,
    pub refreshed_files: Vec<PathBuf>,
    pub partial_files: Vec<PathBuf>,
    pub allow_partial: bool,
}

#[derive(Debug, thiserror::Error)]
pub enum CoreError {
    #[error(transparent)]
    File(#[from] compass_files::FileError),
    #[error(transparent)]
    Extract(#[from] compass_languages::ExtractError),
    #[error(transparent)]
    Graph(#[from] compass_model::GraphError),
    #[error(transparent)]
    Dedup(#[from] compass_graph::DedupError),
    #[error(transparent)]
    Output(#[from] compass_output::OutputError),
    #[error("invalid cached AST extraction for {path}: {source}")]
    InvalidCache {
        path: PathBuf,
        #[source]
        source: serde_json::Error,
    },
    #[error("could not serialize AST extraction for {path}: {source}")]
    SerializeExtraction {
        path: PathBuf,
        #[source]
        source: serde_json::Error,
    },
    #[error("scan root does not exist: {0}")]
    MissingRoot(PathBuf),
    #[error("graph is empty — deterministic extraction produced no nodes")]
    EmptyGraph,
    #[error("diagnostic input must be a JSON object")]
    InvalidDiagnostic,
    #[error("{0}")]
    DiagnosticFile(String),
    #[error("invalid semantic extraction fragment: {0}")]
    InvalidSemanticFragment(serde_json::Error),
    #[error("invalid supplemental extraction fragment: {0}")]
    InvalidSupplementalFragment(serde_json::Error),
    #[error("could not create an AST worker pool: {0}")]
    WorkerPool(String),
    #[error("build worker panicked during {0}")]
    WorkerPanic(String),
    #[error(
        "semantic extraction was incomplete and would shrink the graph ({new} < {existing} nodes)"
    )]
    IncompleteSemanticShrink { existing: usize, new: usize },
    #[error("semantic extraction was incomplete and the existing graph is unreadable: {0}")]
    IncompleteSemanticExisting(PathBuf),
    #[error(transparent)]
    ProgramProvider(#[from] compass_program::ProviderError),
    #[error(transparent)]
    ProgramMerge(#[from] compass_program::MergeError),
    #[error(transparent)]
    ProgramAnalysis(#[from] compass_analysis::AnalysisError),
    #[error(transparent)]
    ProgramIr(#[from] compass_ir::IrError),
    #[error(transparent)]
    Store(#[from] compass_store::StoreError),
    #[error(transparent)]
    Snapshot(#[from] compass_graph::SnapshotError),
    #[error("invalid Program IR input: {0}")]
    InvalidProgramInput(String),
    #[error("invalid completed-build state: {0}")]
    InvalidBuildState(String),
    #[error("precomputed detection root does not match build root")]
    DetectionRootMismatch,
}

/// Run the complete deterministic local graph pipeline without invoking Python,
/// an LLM, a network service, or a dynamically installed grammar.
pub fn build_local_graph(options: &BuildOptions) -> Result<BuildResult, CoreError> {
    build_graph(options, None, &[], None, None)
}

/// Merge a completed semantic provider result into the native graph pipeline.
pub fn build_graph_with_semantic(
    options: &BuildOptions,
    semantic: &SemanticLayer,
) -> Result<BuildResult, CoreError> {
    build_graph(options, Some(semantic), &[], None, None)
}

/// Merge deterministic supplemental facts, such as Cargo or database schema
/// introspection, into the same atomic native graph build.
pub fn build_graph_with_layers(
    options: &BuildOptions,
    semantic: Option<&SemanticLayer>,
    supplemental: &[serde_json::Value],
) -> Result<BuildResult, CoreError> {
    let supplemental = supplemental
        .iter()
        .cloned()
        .map(serde_json::from_value)
        .collect::<Result<Vec<Extraction>, _>>()
        .map_err(CoreError::InvalidSupplementalFragment)?;
    build_graph(options, semantic, &supplemental, None, None)
}

/// Build and retain typed authoritative artifacts for an in-process history
/// publisher, avoiding a filesystem serialization/deserialization handoff.
pub fn build_graph_with_layers_retained(
    options: &BuildOptions,
    semantic: Option<&SemanticLayer>,
    supplemental: &[serde_json::Value],
) -> Result<(BuildResult, RetainedBuildArtifacts), CoreError> {
    let supplemental = supplemental
        .iter()
        .cloned()
        .map(serde_json::from_value)
        .collect::<Result<Vec<Extraction>, _>>()
        .map_err(CoreError::InvalidSupplementalFragment)?;
    let (result, retained) = build_graph_inner(options, semantic, &supplemental, None, None, true)?;
    let retained = retained.ok_or_else(|| {
        CoreError::InvalidBuildState(
            "build completed before authoritative artifacts were available".to_owned(),
        )
    })?;
    Ok((result, retained))
}

pub fn build_graph_with_layers_and_progress(
    options: &BuildOptions,
    semantic: Option<&SemanticLayer>,
    supplemental: &[serde_json::Value],
    progress: &(dyn Fn(BuildFileProgress) + Sync),
) -> Result<BuildResult, CoreError> {
    let supplemental = supplemental
        .iter()
        .cloned()
        .map(serde_json::from_value)
        .collect::<Result<Vec<Extraction>, _>>()
        .map_err(CoreError::InvalidSupplementalFragment)?;
    build_graph(options, semantic, &supplemental, None, Some(progress))
}

pub fn build_graph_with_layers_and_tiebreaker(
    options: &BuildOptions,
    semantic: Option<&SemanticLayer>,
    supplemental: &[serde_json::Value],
    tiebreaker: &mut dyn EntityTiebreaker,
) -> Result<BuildResult, CoreError> {
    let supplemental = supplemental
        .iter()
        .cloned()
        .map(serde_json::from_value)
        .collect::<Result<Vec<Extraction>, _>>()
        .map_err(CoreError::InvalidSupplementalFragment)?;
    build_graph(options, semantic, &supplemental, Some(tiebreaker), None)
}

fn build_graph(
    options: &BuildOptions,
    semantic: Option<&SemanticLayer>,
    supplemental: &[Extraction],
    tiebreaker: Option<&mut dyn EntityTiebreaker>,
    progress: Option<&(dyn Fn(BuildFileProgress) + Sync)>,
) -> Result<BuildResult, CoreError> {
    build_graph_inner(options, semantic, supplemental, tiebreaker, progress, false)
        .map(|(result, _)| result)
}

fn build_graph_inner(
    options: &BuildOptions,
    semantic: Option<&SemanticLayer>,
    supplemental: &[Extraction],
    tiebreaker: Option<&mut dyn EntityTiebreaker>,
    progress: Option<&(dyn Fn(BuildFileProgress) + Sync)>,
    retain_artifacts: bool,
) -> Result<(BuildResult, Option<RetainedBuildArtifacts>), CoreError> {
    let worker_count = pipeline_rayon_workers(options);
    let pool = rayon::ThreadPoolBuilder::new()
        .num_threads(worker_count)
        .thread_name(|index| format!("compass-pipeline-{index}"))
        .build()
        .map_err(|error| CoreError::WorkerPool(error.to_string()))?;
    pool.install(move || {
        build_graph_inner_unscoped(
            options,
            semantic,
            supplemental,
            tiebreaker,
            progress,
            retain_artifacts,
        )
    })
}

fn pipeline_rayon_workers(options: &BuildOptions) -> usize {
    options
        .max_workers
        .unwrap_or_else(default_pipeline_rayon_workers)
        .max(1)
}

fn default_pipeline_rayon_workers() -> usize {
    available_worker_count().clamp(1, PIPELINE_RAYON_WORKER_CAP)
}

fn build_graph_inner_unscoped(
    options: &BuildOptions,
    semantic: Option<&SemanticLayer>,
    supplemental: &[Extraction],
    tiebreaker: Option<&mut dyn EntityTiebreaker>,
    progress: Option<&(dyn Fn(BuildFileProgress) + Sync)>,
    retain_artifacts: bool,
) -> Result<(BuildResult, Option<RetainedBuildArtifacts>), CoreError> {
    let mut timings = BuildTimings::default();
    let mut stage_started = Instant::now();
    if !options.root.exists() {
        return Err(CoreError::MissingRoot(options.root.clone()));
    }
    let root = fs::canonicalize(&options.root).map_err(|source| compass_files::FileError::Io {
        path: options.root.clone(),
        source,
    })?;
    let output_name = std::env::var("COMPASS_OUT").unwrap_or_else(|_| "compass-out".to_owned());
    let output_root = options
        .output_root
        .as_deref()
        .map_or_else(|| root.clone(), absolutize);
    let reuse_cached_analysis = cache_reuse_enabled(options.force, options.reuse_cache_on_force);
    let read_prior_published_graph = prior_published_graph_input_enabled(options.force);
    let output_container = output_root.join(&output_name);
    fs::create_dir_all(&output_container).map_err(|source| compass_files::FileError::Io {
        path: output_container.clone(),
        source,
    })?;
    let prior_build_complete = BuildGuard::ensure_complete(&output_container).is_ok();
    let guard = BuildGuard::begin_excluding(&output_container, &STORE_SNAPSHOT_EXCLUSIONS)?;
    let output_dir = guard.staging_directory().to_path_buf();
    if !options.program_analysis {
        remove_if_exists(&output_dir.join("program.json"))?;
    }
    if options.force || !prior_build_complete {
        remove_if_exists(&output_dir.join(BUILD_STATE_FILE))?;
    }
    let manifest_path = output_dir.join("manifest.json");
    let prior_manifest = Manifest::load(&manifest_path, Some(&root));
    let prior_fact_digest_state = load_ast_fact_digest_state(&output_dir);
    let detect_options = DetectOptions {
        scan_filesystem: options.scan_filesystem,
        gitignore: options.gitignore,
        ignore_policy: options.ignore_policy,
        extra_excludes: options.extra_excludes.clone(),
        scope: options.scope.clone(),
        output_name: output_name.clone(),
        cache_root: Some(output_root.clone()),
        ..DetectOptions::default()
    };
    let mut detection = if let Some(detection) = options.precomputed_detection.clone() {
        let scan_root = fs::canonicalize(&detection.scan_root).map_err(|source| {
            compass_files::FileError::Io {
                path: PathBuf::from(&detection.scan_root),
                source,
            }
        })?;
        if scan_root != root {
            return Err(CoreError::DetectionRootMismatch);
        }
        detection
    } else {
        detect(&root, &detect_options)?
    };
    if options.google_workspace {
        let converted_dir = root.join(&output_name).join("converted");
        let mut sidecars = Vec::new();
        let mut failures = Vec::new();
        for shortcut in &detection.google_workspace_shortcuts {
            match compass_google_workspace::convert_google_workspace_file(shortcut, &converted_dir)
            {
                Ok(Some(sidecar)) => sidecars.push(sidecar),
                Ok(None) => failures.push(format!(
                    "{} [Google Workspace export produced no readable text]",
                    shortcut.display()
                )),
                Err(error) => failures.push(format!(
                    "{} [Google Workspace export failed: {error}]",
                    shortcut.display()
                )),
            }
        }
        detection = detect(
            &root,
            &DetectOptions {
                google_workspace: true,
                additional_files: sidecars,
                ..detect_options
            },
        )?;
        detection.skipped_sensitive.extend(failures);
    }
    timings.detect = stage_started.elapsed();
    stage_started = Instant::now();
    let mut internal_started = Instant::now();
    let preserve_prior_semantic =
        prior_semantic_layer_required(read_prior_published_graph, &output_dir);
    let mut semantic_documents = if preserve_prior_semantic {
        semantic_document_sources(&output_dir.join("graph.json"), &root)
    } else {
        HashSet::new()
    };
    if let Some(layer) = semantic {
        semantic_documents.extend(canonical_source_set(&layer.refreshed_files, &root));
    }
    let mut sources = detection
        .files
        .get("code")
        .into_iter()
        .flatten()
        .map(PathBuf::from)
        .filter(|path| Registry::resolve(path).is_some())
        .collect::<Vec<_>>();
    if !options.code_only {
        sources.extend(
            detection
                .files
                .get("document")
                .into_iter()
                .flatten()
                .map(PathBuf::from)
                .filter(|path| {
                    let structural_document = Registry::resolve(path).is_some_and(|spec| {
                        matches!(spec.kind, ExtractorKind::Markdown | ExtractorKind::Html)
                    });
                    Registry::resolve(path).is_some()
                        && (structural_document
                            || !semantic_documents.contains(&canonical_identity(path)))
                }),
        );
    }

    let reusable_semantic_layer = semantic.is_none()
        || (options.purpose == BuildPurpose::Extract
            && semantic.is_some_and(semantic_layer_is_empty));
    let manifest_unchanged = read_prior_published_graph
        && prior_manifest.is_unchanged(&detection.files, ManifestKind::Ast);
    let build_profile = build_profile(options);
    let profile_digest = build_profile_digest(&build_profile, &output_dir)?;
    let has_program_artifacts =
        options.program_analysis && program_artifact_count(&root, options)? != 0;
    let verified_state = if reusable_semantic_layer && supplemental.is_empty() && manifest_unchanged
    {
        load_verified(
            &output_dir,
            &build_profile,
            &manifest_path,
            prior_build_complete,
        )?
    } else {
        None
    };
    let verified_output =
        verified_state.is_some() && storage_artifacts_complete(options.graph_storage, &output_dir);
    if !has_program_artifacts {
        let verified = verified_state;
        if let Some(state) = verified.filter(|state| state.stats.files == sources.len()) {
            if options.no_viz {
                remove_if_exists(&output_dir.join("graph.html"))?;
            }
            if options.no_cluster {
                remove_if_exists(&output_dir.join(GRAPH_OVERVIEW_FILE))?;
            }
            remove_if_exists(&output_dir.join("needs_update"))?;
            let store_ready = storage_artifacts_complete(options.graph_storage, &output_dir);
            let published_output_dir = commit_snapshot(
                guard,
                &output_container,
                options.graph_storage,
                store_ready,
                false,
                false,
                &mut timings,
            )?;
            return Ok((
                BuildResult {
                    root,
                    output_dir: published_output_dir,
                    detection,
                    files_considered: state.stats.files,
                    files_extracted: 0,
                    files_cached: state.stats.files,
                    empty_files: Vec::new(),
                    nodes: state.stats.nodes,
                    edges: state.stats.edges,
                    communities: state.stats.communities,
                    omitted_nodes: state.stats.omitted_nodes,
                    omitted_edges: state.stats.omitted_edges,
                    identity_collisions: state.stats.identity_collisions,
                    partial_graph: state.stats.omitted_nodes > 0
                        || state.stats.omitted_edges > 0
                        || state.stats.identity_collisions > 0,
                    html_written: output_dir.join("graph.html").is_file(),
                    outputs_changed: false,
                    program_modules: state.stats.program_modules,
                    program_summaries: state.stats.program_summaries,
                    program_syntax_analyzed: 0,
                    program_syntax_reused: state.stats.program_modules,
                    program_artifacts_loaded: 0,
                    program_artifacts_reused: 0,
                    program_artifact_documents_analyzed: 0,
                    program_artifact_documents_reused: 0,
                    program_conflicts: state.stats.program_conflicts,
                    timings,
                },
                None,
            ));
        }
    }
    let unchanged_program_build = if options.program_analysis
        && reusable_semantic_layer
        && supplemental.is_empty()
        && manifest_unchanged
        && verified_output
    {
        load_current_program(&root, &sources, options, &output_dir)?
    } else {
        None
    };
    let unchanged_program_available = unchanged_program_build.is_some();
    let unchanged_program = unchanged_program_build
        .map(|program| ProgramBuildSummary::from_program(program, retain_artifacts));
    if reusable_semantic_layer
        && supplemental.is_empty()
        && manifest_unchanged
        && verified_output
        && (!options.program_analysis || unchanged_program_available)
        && let Some(stats) = unchanged_output_stats(options, &output_dir)
    {
        if options.no_viz {
            remove_if_exists(&output_dir.join("graph.html"))?;
        }
        if options.no_cluster {
            remove_if_exists(&output_dir.join(GRAPH_OVERVIEW_FILE))?;
        }
        remove_if_exists(&output_dir.join("needs_update"))?;
        publish_build_state(
            options,
            &output_dir,
            &manifest_path,
            sources.len(),
            stats.nodes,
            stats.edges,
            stats.communities,
            stats.omissions(),
            unchanged_program.as_ref(),
            None,
            storage_artifacts_complete(options.graph_storage, &output_dir),
            &mut timings,
        )?;
        let published_output_dir = commit_snapshot(
            guard,
            &output_container,
            options.graph_storage,
            true,
            false,
            false,
            &mut timings,
        )?;
        return Ok((
            BuildResult {
                root,
                output_dir: published_output_dir,
                detection,
                files_considered: sources.len(),
                files_extracted: 0,
                files_cached: sources.len(),
                empty_files: Vec::new(),
                nodes: stats.nodes,
                edges: stats.edges,
                communities: stats.communities,
                omitted_nodes: stats.omitted_nodes,
                omitted_edges: stats.omitted_edges,
                identity_collisions: stats.identity_collisions,
                partial_graph: stats.omissions().is_partial(),
                html_written: output_dir.join("graph.html").is_file(),
                outputs_changed: false,
                program_modules: program_modules(unchanged_program.as_ref()),
                program_summaries: program_summaries(unchanged_program.as_ref()),
                program_syntax_analyzed: 0,
                program_syntax_reused: unchanged_program
                    .as_ref()
                    .map_or(0, |program| program.syntax_reused),
                program_artifacts_loaded: 0,
                program_artifacts_reused: unchanged_program
                    .as_ref()
                    .map_or(0, |program| program.artifacts_reused),
                program_artifact_documents_analyzed: 0,
                program_artifact_documents_reused: unchanged_program
                    .as_ref()
                    .map_or(0, |program| program.artifact_documents_reused),
                program_conflicts: unchanged_program
                    .as_ref()
                    .map_or(0, |program| program.conflicts),
                timings,
            },
            None,
        ));
    }

    let output_cache_root = (output_root != root).then_some(output_root.as_path());
    let cache_options = options.cache_root.as_deref().map_or_else(
        || CacheOptions::output_directory(output_cache_root),
        CacheOptions::shared_history,
    );
    let project_evidence = Arc::new(ProjectEvidenceIndex::build(&root, &sources));
    let project_evidence_digest = project_evidence_digest(&project_evidence, &sources, &root);
    let early_missing = fact_neutral_pre_cache_sources(
        options,
        semantic,
        supplemental,
        retain_artifacts,
        prior_fact_digest_state.as_ref(),
        &profile_digest,
        &project_evidence_digest,
        &prior_manifest,
        &detection.files,
        &sources,
        preserve_prior_semantic,
        &root,
    );
    if let Some(early_missing) = early_missing
        && let Some(early_batch) =
            extract_fact_neutral_sources(&early_missing, options, &root, &project_evidence)?
        && let Some(prior_state) = prior_fact_digest_state.as_ref()
    {
        let FactNeutralExtractionBatch {
            extractions: early_extractions,
            source_digests: early_source_digests,
            empty_files: early_empty_files,
        } = early_batch;
        let mut entries = prior_state.entries.clone();
        for (path, digest) in
            ast_fact_digest_entries(&early_missing, &early_extractions, &output_dir, &root)?
        {
            entries.insert(path, digest);
        }
        let current_fact_state = AstFactDigestState {
            schema: AST_FACT_DIGESTS_SCHEMA.to_owned(),
            profile_digest: profile_digest.clone(),
            project_evidence_digest: project_evidence_digest.clone(),
            entries,
        };
        if early_missing.iter().all(|path| {
            let key = relative_fact_path(path, &root);
            prior_state.entries.get(&key) == current_fact_state.entries.get(&key)
        }) {
            let inventory = detection_inventory(
                &detection,
                semantic,
                &BTreeMap::new(),
                &BTreeMap::new(),
                &root,
            );
            let configuration_digest = graph_configuration_digest(options, &output_dir)?;
            if let Some((previous, current, changed_file_node_ids)) = prepare_fact_neutral_document(
                &output_dir,
                &root,
                &early_source_digests,
                inventory,
                &early_extractions,
                configuration_digest,
                options.max_source_bytes,
                options.graph_storage.publishes_store(),
            )? {
                let mut cache = Cache::open(&root, cache_options)?;
                let cache_sources = early_missing
                    .iter()
                    .zip(&early_extractions)
                    .collect::<Vec<_>>();
                cache.write_portable_ast_batch_ref(&cache_sources)?;
                cache.flush()?;
                timings.deterministic_extract = stage_started.elapsed();
                let result = publish_fact_neutral_incremental(
                    options,
                    root,
                    output_dir,
                    &output_container,
                    guard,
                    &manifest_path,
                    prior_manifest,
                    detection,
                    &sources,
                    &early_missing,
                    early_empty_files,
                    previous.as_ref(),
                    &current,
                    &changed_file_node_ids,
                    &current_fact_state,
                    &mut timings,
                )?;
                return Ok((result, None));
            }
        }
    }
    let mut cache = Cache::open(&root, cache_options)?;
    let mut extractions = BTreeMap::<PathBuf, Extraction>::new();
    let mut missing = Vec::new();
    if reuse_cached_analysis {
        for path in &sources {
            if fs::metadata(path).is_ok_and(|metadata| {
                metadata.is_file() && metadata.len() > options.max_source_bytes
            }) {
                extractions.insert(
                    path.clone(),
                    oversized_source_extraction(path, options.max_source_bytes)?,
                );
                continue;
            }
            let cached =
                cache.load_portable_ast_typed(path, false, |extraction: &Extraction| {
                    extraction
                        .extensions
                        .get(EXTRACTION_QUALITY_PARTIAL)
                        .and_then(Value::as_bool)
                        == Some(true)
                })?;
            if let Some(extraction) = cached {
                if cached_framework_evidence_matches(&extraction, path, &project_evidence)
                    && cached_universal_evidence_matches(&extraction, path)
                {
                    extractions.insert(path.clone(), extraction);
                } else {
                    missing.push(path.clone());
                }
            } else {
                missing.push(path.clone());
            }
        }
    } else {
        for path in &sources {
            if fs::metadata(path).is_ok_and(|metadata| {
                metadata.is_file() && metadata.len() > options.max_source_bytes
            }) {
                extractions.insert(
                    path.clone(),
                    oversized_source_extraction(path, options.max_source_bytes)?,
                );
            } else {
                missing.push(path.clone());
            }
        }
    }
    let worker_count = options
        .max_workers
        .unwrap_or_else(|| default_ast_workers(missing.len()));
    // Resolver source text is only consulted by the PHP type-reference pass.
    // Keeping every decoded source string alive across extraction and graph
    // publication otherwise duplicates the repository's source footprint in
    // memory for languages whose resolution is entirely fact-based.
    let needs_resolver_source_text = sources.iter().any(|source| {
        source
            .extension()
            .is_some_and(|extension| extension.eq_ignore_ascii_case("php"))
    });
    // An explicit worker count is an opt-in performance decision. Honor it
    // even for smaller repositories so callers can trade parser-table
    // residency for throughput instead of silently falling back to the
    // sequential path below. The automatic path uses the same bounded local
    // pool once enough missing files exist to amortize parser-table residency.
    // On small repositories, a single requested worker remains sequential; it
    // avoids paying for a pool that cannot add parallelism.
    let parallel_extraction = should_parallel_extract(options, missing.len());
    let worker_pool = if parallel_extraction {
        Some(
            rayon::ThreadPoolBuilder::new()
                .num_threads(worker_count)
                .thread_name(|index| format!("compass-ast-{index}"))
                .build()
                .map_err(|error| CoreError::WorkerPool(error.to_string()))?,
        )
    } else {
        None
    };
    profile_internal("extract setup and cache load", &mut internal_started);
    // A Rayon worker pool costs more resident memory than it saves time when
    // only a handful of files are missing. Below the measured crossover stay
    // sequential; above it use a bounded local pool so an embedding
    // application's global Rayon settings cannot silently serialize cold
    // extraction or multiply parser working sets without an explicit opt-in.
    let completed_files = Mutex::new(0_usize);
    let total_files = missing.len();
    let extract_source =
        |engine: &mut Engine, path: &PathBuf| -> Result<_, compass_languages::ExtractError> {
            let metadata = fs::metadata(path).map_err(|source| compass_files::FileError::Io {
                path: path.clone(),
                source,
            })?;
            if metadata.len() > options.max_source_bytes {
                let graph = oversized_source_extraction(path, options.max_source_bytes)?;
                if let Some(progress) = progress {
                    let mut completed = completed_files
                        .lock()
                        .unwrap_or_else(|poisoned| poisoned.into_inner());
                    *completed += 1;
                    progress(BuildFileProgress {
                        current: *completed,
                        total: total_files,
                        path: path.clone(),
                    });
                }
                return Ok((
                    path.clone(),
                    graph,
                    (path.to_string_lossy().into_owned(), String::new()),
                    None,
                    None,
                ));
            }
            let bytes = fs::read(path).map_err(|source| compass_files::FileError::Io {
                path: path.clone(),
                source,
            })?;
            let content_hash = cache.content_hash_from_bytes(path, &bytes);
            let source_digest = SourceDigest {
                content_digest: format!("sha256:{:x}", Sha256::digest(&bytes)),
                byte_size: bytes.len() as u64,
            };
            let byte_len = bytes.len() as u64;
            let source_file = path
                .strip_prefix(&root)
                .unwrap_or(path)
                .to_string_lossy()
                .replace('\\', "/");
            let language = Registry::resolve(path).map_or("", |spec| spec.name);
            let (mut graph, program) = if options.program_analysis {
                let combined = engine.extract_source_combined(path, &source_file, &bytes)?;
                (combined.graph, combined.program)
            } else {
                (
                    engine.extract_source_graph_only(path, &source_file, &bytes)?,
                    None,
                )
            };
            let empty_structured_document = bytes.is_empty()
                && Registry::resolve(path).is_some_and(|spec| {
                    matches!(
                        spec.kind,
                        ExtractorKind::JsonConfig | ExtractorKind::ProjectXml | ExtractorKind::Xaml
                    )
                });
            if empty_structured_document && graph.error.is_none() {
                graph.error = Some(format!("{language} extraction failed: empty document"));
            }
            compact_extraction(&mut graph);
            let source = if needs_resolver_source_text && is_php_source_path(path) {
                (
                    path.to_string_lossy().into_owned(),
                    String::from_utf8_lossy(&bytes).into_owned(),
                )
            } else {
                (path.to_string_lossy().into_owned(), String::new())
            };
            let prepared = program.map(|batch| PreparedSyntaxInput {
                source_file,
                language: language.to_owned(),
                bytes,
                batch,
            });
            if let Some(progress) = progress {
                let mut completed = completed_files
                    .lock()
                    .unwrap_or_else(|poisoned| poisoned.into_inner());
                *completed += 1;
                progress(BuildFileProgress {
                    current: *completed,
                    total: total_files,
                    path: path.clone(),
                });
            }
            Ok((
                path.clone(),
                graph,
                source,
                prepared,
                Some((
                    content_hash,
                    byte_len,
                    metadata.modified().ok(),
                    source_digest,
                )),
            ))
        };
    let fresh_outcomes = if !parallel_extraction {
        let mut engine = Engine::with_project_evidence(Arc::clone(&project_evidence));
        missing
            .iter()
            .map(|path| extract_source(&mut engine, path))
            .collect::<Vec<_>>()
    } else {
        let worker_evidence = Arc::clone(&project_evidence);
        let extract = || {
            missing
                .par_iter()
                .map_init(
                    || Engine::with_project_evidence(Arc::clone(&worker_evidence)),
                    extract_source,
                )
                .collect::<Vec<_>>()
        };
        if let Some(pool) = &worker_pool {
            pool.install(extract)
        } else {
            extract()
        }
    };
    let mut extraction_failures = BTreeMap::new();
    let mut fresh = missing
        .iter()
        .cloned()
        .zip(fresh_outcomes)
        .filter_map(|(path, outcome)| match outcome {
            Ok(extracted) => Some(extracted),
            Err(error) => {
                extraction_failures.insert(
                    canonical_identity(&path),
                    portable_diagnostic_reason(&error.to_string(), &path, &root),
                );
                None
            }
        })
        .collect::<Vec<_>>();
    let mut extraction_partials = BTreeMap::new();
    for (path, extraction) in &mut extractions {
        prepare_extraction_for_publication(
            path,
            extraction,
            &root,
            &mut extraction_failures,
            &mut extraction_partials,
        );
    }
    for (path, extraction, _, _, _) in &mut fresh {
        prepare_extraction_for_publication(
            path,
            extraction,
            &root,
            &mut extraction_failures,
            &mut extraction_partials,
        );
    }
    profile_internal("tree-sitter extraction", &mut internal_started);
    let prepared = if !reuse_cached_analysis {
        fresh
            .iter_mut()
            .filter_map(|(_, _, _, prepared, _)| prepared.take())
            .collect::<Vec<_>>()
    } else {
        Vec::new()
    };
    let mut program_handle = if options.program_analysis {
        let program_root = root.clone();
        let program_sources = sources.clone();
        let mut program_options = options.clone();
        program_options.force = !reuse_cached_analysis;
        let program_cache_root = options.cache_root.clone();
        let program_output_cache_root = output_cache_root.map(Path::to_path_buf);
        let program_output_dir = output_dir.clone();
        Some(
            std::thread::Builder::new()
                .name("compass-program".to_owned())
                .spawn(move || {
                    let started = Instant::now();
                    let program_cache_options = program_cache_root.as_deref().map_or_else(
                        || CacheOptions::output_directory(program_output_cache_root.as_deref()),
                        CacheOptions::shared_history,
                    );
                    let program_cache =
                        Cache::open(&program_root, program_cache_options)?.without_hash_flush();
                    let program = build_program(
                        &program_root,
                        &program_sources,
                        &program_options,
                        &program_cache,
                        prepared,
                    )?;
                    let summary = if retain_artifacts {
                        let canonical_started = Instant::now();
                        let program_seal = write_program(&program_output_dir, &program.analysis)?;
                        profile_internal_duration(
                            "Program canonical JSON",
                            canonical_started.elapsed(),
                        );
                        ProgramBuildSummary::from_program_with_seal(program, true, program_seal)
                    } else {
                        let ProgramBuild {
                            analysis,
                            canonical_bytes: _,
                            syntax_analyzed,
                            syntax_reused,
                            artifacts_loaded,
                            artifacts_reused,
                            artifact_documents_analyzed,
                            artifact_documents_reused,
                            conflicts,
                            compiler_projection,
                        } = program;
                        let modules = analysis.program.modules.len();
                        let summaries = analysis.summaries.len();
                        let providers = analysis.program.providers.len();
                        let writer_output_dir = program_output_dir.clone();
                        let writer = std::thread::Builder::new()
                            .name("compass-program-json".to_owned())
                            .spawn(move || {
                                let canonical_started = Instant::now();
                                let seal = write_program(&writer_output_dir, &analysis)?;
                                let elapsed = canonical_started.elapsed();
                                profile_internal_duration("Program canonical JSON", elapsed);
                                Ok::<_, CoreError>((seal, elapsed))
                            })
                            .map_err(|error| CoreError::WorkerPool(error.to_string()))?;
                        ProgramBuildSummary {
                            seal: None,
                            pending_seal: Some(writer),
                            modules,
                            summaries,
                            providers,
                            syntax_analyzed,
                            syntax_reused,
                            artifacts_loaded,
                            artifacts_reused,
                            artifact_documents_analyzed,
                            artifact_documents_reused,
                            conflicts,
                            compiler_projection: Some(compiler_projection),
                            analysis: None,
                        }
                    };
                    Ok::<_, CoreError>((summary, started.elapsed()))
                })
                .map_err(|error| CoreError::WorkerPool(error.to_string()))?,
        )
    } else {
        None
    };
    let mut empty_files = Vec::new();
    let fresh_paths = fresh
        .iter()
        .map(|(path, _, _, _, _)| path.clone())
        .collect::<HashSet<_>>();
    let mut fresh_source_text = HashMap::with_capacity(fresh.len());
    let mut fresh_source_digests = BTreeMap::new();
    for (path, extraction, source, _, content_hash) in fresh {
        if !extraction_has_cacheable_ast_facts(&extraction) {
            empty_files.push(path.clone());
        }
        if let Some((hash, size, modified, source_digest)) = content_hash {
            cache.seed_content_hash(&path, hash, size, modified)?;
            let relative_source = path
                .strip_prefix(&root)
                .unwrap_or(&path)
                .to_string_lossy()
                .replace('\\', "/");
            fresh_source_digests.insert(relative_source, source_digest);
        }
        let (source_path, source) = source;
        fresh_source_text.insert(source_path, source);
        extractions.insert(path, extraction);
    }
    profile_internal("AST extraction collation", &mut internal_started);

    let ordered_paths = sources
        .iter()
        .filter(|path| extractions.contains_key(*path))
        .cloned()
        .collect::<Vec<_>>();
    let mut ordered = sources
        .iter()
        .filter_map(|path| extractions.remove(path))
        .collect::<Vec<_>>();
    let ast_root_marker = format!("{}_", make_id(&[&root.to_string_lossy()]));
    let portable_check_started = Instant::now();
    let ast_is_portable = ast_extractions_are_portable(&ordered, &root);
    profile_internal_duration("portable AST precheck", portable_check_started.elapsed());
    let ast_id_remap = if ast_is_portable {
        AHashMap::new()
    } else {
        let identity_map_started = Instant::now();
        let live_id_remap = ast_source_identity_map(&sources, &root);
        profile_internal_duration(
            "portable AST source identity map",
            identity_map_started.elapsed(),
        );
        let remap_collection_started = Instant::now();
        let remap = collect_ast_id_remap(&ordered, &root, &live_id_remap);
        profile_internal_duration(
            "portable AST remap collection",
            remap_collection_started.elapsed(),
        );
        remap
    };
    if !ast_id_remap.is_empty() {
        let remap_application_started = Instant::now();
        if ordered.len() < 256 {
            ordered.iter_mut().for_each(|extraction| {
                apply_ast_id_remap(extraction, &ast_id_remap, &ast_root_marker);
            });
        } else {
            ordered.par_iter_mut().for_each(|extraction| {
                apply_ast_id_remap(extraction, &ast_id_remap, &ast_root_marker);
            });
        }
        profile_internal_duration(
            "portable AST remap application",
            remap_application_started.elapsed(),
        );
    }
    profile_internal("portable AST ID remapping", &mut internal_started);
    // Continue the cold build from the same portable representation that a
    // subsequent cache hit will decode. Cache hits already have this
    // representation; only fresh extractions need the normalization walk.
    for (path, extraction) in ordered_paths.iter().zip(&mut ordered) {
        if fresh_paths.contains(path) {
            prepare_portable_ast_cache_entry(extraction, path, &root);
        }
    }
    let ast_cache_sources = ordered_paths
        .iter()
        .zip(&ordered)
        .filter(|(path, extraction)| {
            fresh_paths.contains(*path) && extraction_has_cacheable_ast_facts(extraction)
        })
        .collect::<Vec<_>>();
    let ((cache_result, cache_elapsed), (premerge_digest_result, digest_elapsed)) = rayon::join(
        || {
            let started = Instant::now();
            let result = cache
                .write_portable_ast_batch_ref(&ast_cache_sources)
                .and_then(|()| cache.flush());
            (result, started.elapsed())
        },
        || {
            let started = Instant::now();
            let result = ast_fact_digest_entries(&ordered_paths, &ordered, &output_dir, &root);
            (result, started.elapsed())
        },
    );
    cache_result?;
    let premerge_fact_digests = premerge_digest_result?;
    drop(ast_cache_sources);
    profile_internal_duration("AST cache streaming publication", cache_elapsed);
    profile_internal_duration("AST fact digest construction", digest_elapsed);
    // Declaration/definition merging is project-wide and is not idempotent.
    // Cache the portable per-file facts before applying it so a warm build
    // executes the same single merge as a cold build.
    let declaration_merge_changed =
        merge_decl_def_classes_if_needed_changed(&mut ordered, &ordered_paths);
    profile_internal("declaration merge", &mut internal_started);
    let live_sources = detection
        .files
        .values()
        .flatten()
        .map(|path| canonical_identity(Path::new(path)))
        .collect::<HashSet<_>>();
    let source_removed = prior_manifest
        .entries()
        .keys()
        .map(|path| canonical_identity(Path::new(path)))
        .any(|path| !live_sources.contains(&path));
    profile_internal("incremental source inventory", &mut internal_started);
    let fact_digest_started = Instant::now();
    let fact_digest_entries = if declaration_merge_changed {
        ast_fact_digest_entries(&ordered_paths, &ordered, &output_dir, &root)?
    } else {
        premerge_fact_digests
    };
    profile_internal_duration(
        "AST fact digest post-merge refresh",
        fact_digest_started.elapsed(),
    );
    let current_fact_state = AstFactDigestState {
        schema: AST_FACT_DIGESTS_SCHEMA.to_owned(),
        profile_digest: profile_digest.clone(),
        project_evidence_digest: project_evidence_digest.clone(),
        entries: fact_digest_entries,
    };
    internal_started = Instant::now();
    let fact_neutral = fact_neutral_incremental_candidate(
        options,
        semantic,
        supplemental,
        retain_artifacts,
        prior_fact_digest_state.as_ref(),
        &current_fact_state,
        &prior_manifest,
        &detection.files,
        &missing,
        source_removed,
        &root,
    );
    if fact_neutral && !preserve_prior_semantic {
        let inventory = detection_inventory(
            &detection,
            semantic,
            &extraction_failures,
            &extraction_partials,
            &root,
        );
        let configuration_digest = graph_configuration_digest(options, &output_dir)?;
        if let Some((previous, current, changed_file_node_ids)) = prepare_fact_neutral_document(
            &output_dir,
            &root,
            &fresh_source_digests,
            inventory,
            &ordered,
            configuration_digest,
            options.max_source_bytes,
            options.graph_storage.publishes_store(),
        )? {
            timings.deterministic_extract = stage_started.elapsed();
            let result = publish_fact_neutral_incremental(
                options,
                root,
                output_dir,
                &output_container,
                guard,
                &manifest_path,
                prior_manifest,
                detection,
                &sources,
                &missing,
                empty_files,
                previous.as_ref(),
                &current,
                &changed_file_node_ids,
                &current_fact_state,
                &mut timings,
            )?;
            return Ok((result, None));
        }
    }
    let read_source = |path: &PathBuf| read_source_text_with_limit(path, options.max_source_bytes);
    let read_cached_source = |path: &PathBuf| {
        if fresh_paths.contains(path) {
            None
        } else if needs_resolver_source_text && is_php_source_path(path) {
            read_source(path)
        } else {
            // Cross-file resolution uses the keys as the complete language
            // inventory even when a language does not need decoded source
            // text. Preserve that inventory across partial and fully cached
            // rebuilds without retaining duplicate source contents.
            Some((path.to_string_lossy().into_owned(), String::new()))
        }
    };
    let cached_source_text: HashMap<_, _> = if sources.len() < 256 {
        sources.iter().filter_map(read_cached_source).collect()
    } else if let Some(pool) = &worker_pool {
        pool.install(|| sources.par_iter().filter_map(read_cached_source).collect())
    } else {
        sources.par_iter().filter_map(read_cached_source).collect()
    };
    fresh_source_text.extend(cached_source_text);
    let source_text = fresh_source_text;
    profile_internal("resolver source inventory", &mut internal_started);
    drop(worker_pool);
    drop(project_evidence);
    drop(fresh_paths);
    drop(ordered_paths);
    drop(ast_id_remap);
    drop(ast_root_marker);
    profile_extraction_inventory(&ordered);
    let program_projection_sites = collect_program_projection_sites(&ordered);
    profile_internal("Program projection site collection", &mut internal_started);
    let resolution_admission = match options.inference_level {
        InferenceLevel::Low => ResolutionAdmission::Low,
        InferenceLevel::Medium => ResolutionAdmission::Medium,
        InferenceLevel::High => ResolutionAdmission::High,
        InferenceLevel::Max => ResolutionAdmission::Max,
    };
    let mut resolved = resolve_prevalidated_owned_with_root_at_inference(
        ordered,
        &source_text,
        &root,
        resolution_admission,
    );
    profile_internal("cross-file resolution total", &mut internal_started);
    drop(source_text);
    internal_started = Instant::now();
    let defer_program_join = options.force && !options.no_cluster && !has_program_artifacts;
    let mut program = if defer_program_join {
        None
    } else {
        join_program_worker(program_handle.take(), &mut timings)?
    };
    profile_internal("wait for Program analysis", &mut internal_started);
    if let Some(program) = program.as_mut()
        && let Some(compiler_projection) = program.compiler_projection.take()
    {
        apply_program_projection(
            &mut resolved,
            &program_projection_sites,
            &compiler_projection,
        );
    }
    drop(program_projection_sites);
    profile_internal("Program graph projection", &mut internal_started);
    finalize_ast_extraction(&mut resolved, &root);
    profile_internal("AST finalization", &mut internal_started);
    timings.deterministic_extract = stage_started.elapsed();
    stage_started = Instant::now();
    if preserve_prior_semantic {
        let refreshed = semantic
            .map(|layer| {
                let mut refreshed = canonical_source_set(&layer.refreshed_files, &root);
                refreshed.extend(stale_semantic_sources(
                    &output_dir.join("graph.json"),
                    &root,
                    &detection.files,
                ));
                refreshed
            })
            .unwrap_or_default();
        preserve_semantic_layer(
            &mut resolved,
            &output_dir.join("graph.json"),
            &root,
            &refreshed,
        );
    }
    if let Some(layer) = semantic {
        let mut extracted: Extraction = serde_json::from_value(layer.fragment.clone())
            .map_err(CoreError::InvalidSemanticFragment)?;
        finalize_semantic_extraction(&mut extracted, &root);
        resolved.nodes.extend(extracted.nodes);
        resolved.edges.extend(extracted.edges);
        resolved.hyperedges.extend(extracted.hyperedges);
    }
    for extracted in supplemental {
        resolved.nodes.extend(extracted.nodes.iter().cloned());
        resolved.edges.extend(extracted.edges.iter().cloned());
        resolved
            .hyperedges
            .extend(extracted.hyperedges.iter().cloned());
    }
    let live_sources = detection
        .files
        .values()
        .flatten()
        .map(|path| canonical_identity(Path::new(path)))
        .collect::<HashSet<_>>();
    let source_removed = prior_manifest
        .entries()
        .keys()
        .map(|path| canonical_identity(Path::new(path)))
        .any(|path| !live_sources.contains(&path));
    if options.no_cluster
        && options.purpose == BuildPurpose::Extract
        && !options.force
        && manifest_unchanged
        && missing.is_empty()
        && !source_removed
        && supplemental.is_empty()
        && semantic.is_some_and(semantic_layer_is_empty)
        && let Ok(document) = GraphDocument::load(&output_dir.join("graph.json"))
    {
        let omissions = saved_publication_omissions(&output_dir);
        let mut manifest = prior_manifest;
        save_build_manifest(
            &mut manifest,
            &detection.files,
            &manifest_path,
            &root,
            semantic,
        )?;
        publish_build_state(
            options,
            &output_dir,
            &manifest_path,
            sources.len(),
            document.nodes.len(),
            document.links.len(),
            0,
            omissions,
            program.as_ref(),
            None,
            storage_artifacts_complete(options.graph_storage, &output_dir),
            &mut timings,
        )?;
        let published_output_dir = commit_snapshot(
            guard,
            &output_container,
            options.graph_storage,
            true,
            false,
            false,
            &mut timings,
        )?;
        return Ok((
            BuildResult {
                root,
                output_dir: published_output_dir,
                detection,
                files_considered: sources.len(),
                files_extracted: 0,
                files_cached: sources.len(),
                empty_files,
                nodes: document.nodes.len(),
                edges: document.links.len(),
                communities: 0,
                omitted_nodes: omissions.nodes,
                omitted_edges: omissions.edges,
                identity_collisions: omissions.identity_collisions,
                partial_graph: omissions.is_partial(),
                html_written: false,
                outputs_changed: false,
                program_modules: program_modules(program.as_ref()),
                program_summaries: program_summaries(program.as_ref()),
                program_syntax_analyzed: program
                    .as_ref()
                    .map_or(0, |program| program.syntax_analyzed),
                program_syntax_reused: program.as_ref().map_or(0, |program| program.syntax_reused),
                program_artifacts_loaded: program
                    .as_ref()
                    .map_or(0, |program| program.artifacts_loaded),
                program_artifacts_reused: program
                    .as_ref()
                    .map_or(0, |program| program.artifacts_reused),
                program_artifact_documents_analyzed: program
                    .as_ref()
                    .map_or(0, |program| program.artifact_documents_analyzed),
                program_artifact_documents_reused: program
                    .as_ref()
                    .map_or(0, |program| program.artifact_documents_reused),
                program_conflicts: program.as_ref().map_or(0, |program| program.conflicts),
                timings,
            },
            None,
        ));
    }
    if options.no_cluster {
        let no_cluster_graph_started = Instant::now();
        enforce_incomplete_raw_guard(
            semantic,
            &output_dir.join("graph.json"),
            &root,
            deduped_node_count(&resolved.nodes),
        )?;
        let document = build_document(
            resolved,
            true,
            true,
            Some(&root),
            tiebreaker,
            options.inference_level,
        )?;
        profile_internal_duration(
            "no-cluster graph document build",
            no_cluster_graph_started.elapsed(),
        );
        let configuration_digest = graph_configuration_digest(options, &output_dir)?;
        let source_commit = options
            .built_at_commit
            .clone()
            .or_else(|| git_commit(&root));
        let no_cluster_normalization_started = Instant::now();
        let mut published =
            normalize_document_v1_with_inventory_and_source_digests_best_effort_owned_at_inference(
                document,
                &root,
                configuration_digest,
                source_commit.as_deref(),
                detection_inventory(
                    &detection,
                    semantic,
                    &extraction_failures,
                    &extraction_partials,
                    &root,
                ),
                Some(&fresh_source_digests),
                options.inference_level,
            )?;
        apply_inference_level(&mut published.document, options.inference_level);
        profile_internal_duration(
            "no-cluster v1 normalization",
            no_cluster_normalization_started.elapsed(),
        );
        if published.document.nodes.is_empty() {
            return Err(CoreError::EmptyGraph);
        }
        let omissions = published.omissions;
        let published_nodes = published.document.nodes.len();
        let published_edges = published.document.links.len();
        let no_cluster_graph_write_started = Instant::now();
        let (store_metrics, graph_seal) = if options.graph_storage.publishes_store() {
            let (metrics, seal) =
                if let Some(previous) = load_graph_delta_base(&output_dir, &published.document) {
                    publish_graph_and_store_delta(&output_dir, &previous, &published.document)?
                } else {
                    publish_graph_and_store_from_canonical(&output_dir, &published.document)?
                };
            (Some(metrics), Some(seal))
        } else {
            let graph_path = output_dir.join("graph.json");
            let receipt = write_atomic_with_digest(&graph_path, |writer| {
                write_canonical_graph_json(&published.document, writer).map_err(|source| {
                    compass_files::FileError::Io {
                        path: graph_path.clone(),
                        source,
                    }
                })
            })?;
            (
                None,
                Some(ArtifactSeal {
                    bytes: receipt.bytes,
                    sha256: receipt.sha256,
                }),
            )
        };
        profile_internal_duration(
            "no-cluster graph and store publication",
            no_cluster_graph_write_started.elapsed(),
        );
        if let Some(metrics) = store_metrics {
            record_store_metrics(&mut timings, metrics);
        }
        remove_if_exists(&output_dir.join(GRAPH_OVERVIEW_FILE))?;
        save_output_stats(
            &output_dir,
            published_nodes,
            published_edges,
            0,
            false,
            omissions,
        )?;
        write_ast_fact_digest_state(&output_dir, &current_fact_state)?;
        write_semantic_marker(&output_dir, semantic)?;
        if options.purpose == BuildPurpose::Update {
            write_text_atomic(
                output_dir.join("source-root.txt"),
                &options.root.to_string_lossy(),
            )?;
        }
        let mut manifest = prior_manifest;
        let no_cluster_manifest_started = Instant::now();
        save_build_manifest(
            &mut manifest,
            &detection.files,
            &manifest_path,
            &root,
            semantic,
        )?;
        profile_internal_duration(
            "no-cluster manifest publication",
            no_cluster_manifest_started.elapsed(),
        );
        remove_if_exists(&output_dir.join("needs_update"))?;
        if let Some(program) = program.as_mut() {
            program.finish_pending_seal(&mut timings)?;
        }
        let no_cluster_state_started = Instant::now();
        publish_build_state(
            options,
            &output_dir,
            &manifest_path,
            sources.len(),
            published_nodes,
            published_edges,
            0,
            omissions,
            program.as_ref(),
            graph_seal,
            store_metrics.is_some(),
            &mut timings,
        )?;
        profile_internal_duration(
            "no-cluster build-state publication",
            no_cluster_state_started.elapsed(),
        );
        let no_cluster_commit_started = Instant::now();
        let published_output_dir = commit_snapshot(
            guard,
            &output_container,
            options.graph_storage,
            true,
            !options.graph_storage.publishes_store(),
            true,
            &mut timings,
        )?;
        profile_internal_duration(
            "no-cluster snapshot commit",
            no_cluster_commit_started.elapsed(),
        );
        timings.publish = stage_started.elapsed();
        return Ok((
            BuildResult {
                root,
                output_dir: published_output_dir,
                detection,
                files_considered: sources.len(),
                files_extracted: missing.len(),
                files_cached: sources.len().saturating_sub(missing.len()),
                empty_files,
                nodes: published_nodes,
                edges: published_edges,
                communities: 0,
                omitted_nodes: omissions.nodes,
                omitted_edges: omissions.edges,
                identity_collisions: omissions.identity_collisions,
                partial_graph: omissions.is_partial(),
                html_written: false,
                outputs_changed: true,
                program_modules: program_modules(program.as_ref()),
                program_summaries: program_summaries(program.as_ref()),
                program_syntax_analyzed: program
                    .as_ref()
                    .map_or(0, |program| program.syntax_analyzed),
                program_syntax_reused: program.as_ref().map_or(0, |program| program.syntax_reused),
                program_artifacts_loaded: program
                    .as_ref()
                    .map_or(0, |program| program.artifacts_loaded),
                program_artifacts_reused: program
                    .as_ref()
                    .map_or(0, |program| program.artifacts_reused),
                program_artifact_documents_analyzed: program
                    .as_ref()
                    .map_or(0, |program| program.artifact_documents_analyzed),
                program_artifact_documents_reused: program
                    .as_ref()
                    .map_or(0, |program| program.artifact_documents_reused),
                program_conflicts: program.as_ref().map_or(0, |program| program.conflicts),
                timings,
            },
            None,
        ));
    }
    let document = build_document(
        resolved,
        true,
        true,
        Some(&root),
        tiebreaker,
        options.inference_level,
    )?;
    profile_internal("graph document build and dedup", &mut internal_started);
    timings.graph_assembly = stage_started.elapsed();
    stage_started = Instant::now();
    if document.nodes.is_empty() {
        return Err(CoreError::EmptyGraph);
    }
    let commit = options.built_at_commit.clone().or_else(|| {
        std::env::current_dir()
            .ok()
            .and_then(|directory| git_commit(&directory))
    });
    let unchanged_artifacts_complete = match options.purpose {
        BuildPurpose::Update => update_artifacts_complete(options, &output_dir),
        BuildPurpose::Extract => {
            output_dir.join("graph.json").is_file()
                && output_dir.join("analysis.json").is_file()
                && storage_artifacts_complete(options.graph_storage, &output_dir)
        }
    };
    let unchanged_layers = semantic.is_none()
        || (options.purpose == BuildPurpose::Extract
            && semantic.is_some_and(semantic_layer_is_empty));
    if unchanged_layers && supplemental.is_empty() && !options.force && unchanged_artifacts_complete
    {
        let preflight_started = Instant::now();
        let mut preflight = normalize_document_v1_with_inventory_best_effort_at_inference(
            &document,
            &root,
            graph_configuration_digest(options, &output_dir)?,
            commit.as_deref(),
            detection_inventory(
                &detection,
                semantic,
                &extraction_failures,
                &extraction_partials,
                &root,
            ),
            options.inference_level,
        )?;
        apply_inference_level(&mut preflight.document, options.inference_level);
        let preflight_document = preflight.document.to_legacy_document()?;
        profile_internal_duration("graph.json v1 preflight", preflight_started.elapsed());
        if !preflight.omissions.is_partial()
            && GraphDocument::load(&output_dir.join("graph.json"))
                .is_ok_and(|existing| topology_is_unchanged(&existing, &preflight_document))
        {
            let communities = previous_communities(&output_dir.join("graph.json"))
                .values()
                .copied()
                .collect::<HashSet<_>>()
                .len();
            let mut manifest = prior_manifest;
            save_build_manifest(
                &mut manifest,
                &detection.files,
                &manifest_path,
                &root,
                semantic,
            )?;
            remove_if_exists(&output_dir.join("needs_update"))?;
            if let Some(program) = program.as_mut() {
                program.finish_pending_seal(&mut timings)?;
            }
            publish_build_state(
                options,
                &output_dir,
                &manifest_path,
                sources.len(),
                preflight_document.nodes.len(),
                preflight_document.links.len(),
                communities,
                PublicationOmissions::default(),
                program.as_ref(),
                None,
                true,
                &mut timings,
            )?;
            let published_output_dir = commit_snapshot(
                guard,
                &output_container,
                options.graph_storage,
                true,
                false,
                false,
                &mut timings,
            )?;
            return Ok((
                BuildResult {
                    root,
                    output_dir: published_output_dir,
                    detection,
                    files_considered: sources.len(),
                    files_extracted: missing.len(),
                    files_cached: sources.len().saturating_sub(missing.len()),
                    empty_files,
                    nodes: preflight_document.nodes.len(),
                    edges: preflight_document.links.len(),
                    communities,
                    omitted_nodes: 0,
                    omitted_edges: 0,
                    identity_collisions: 0,
                    partial_graph: false,
                    html_written: output_dir.join("graph.html").is_file(),
                    outputs_changed: false,
                    program_modules: program_modules(program.as_ref()),
                    program_summaries: program_summaries(program.as_ref()),
                    program_syntax_analyzed: program
                        .as_ref()
                        .map_or(0, |program| program.syntax_analyzed),
                    program_syntax_reused: program
                        .as_ref()
                        .map_or(0, |program| program.syntax_reused),
                    program_artifacts_loaded: program
                        .as_ref()
                        .map_or(0, |program| program.artifacts_loaded),
                    program_artifacts_reused: program
                        .as_ref()
                        .map_or(0, |program| program.artifacts_reused),
                    program_artifact_documents_analyzed: program
                        .as_ref()
                        .map_or(0, |program| program.artifact_documents_analyzed),
                    program_artifact_documents_reused: program
                        .as_ref()
                        .map_or(0, |program| program.artifact_documents_reused),
                    program_conflicts: program.as_ref().map_or(0, |program| program.conflicts),
                    timings,
                },
                None,
            ));
        }
    }

    // Establish the publication boundary before topology analysis. Reports,
    // clustering, and viewer models must all consume the same validated graph
    // that will be written as graph.json.
    let raw_document = document;
    let configuration_digest = graph_configuration_digest(options, &output_dir)?;
    let publication_inventory = detection_inventory(
        &detection,
        semantic,
        &extraction_failures,
        &extraction_partials,
        &root,
    );
    let publication_evidence_started = Instant::now();
    let mut publication_evidence = BuildEvidence::from_document_with_source_digests(
        &root,
        &raw_document,
        configuration_digest,
        &fresh_source_digests,
    )?;
    publication_evidence.include_inventory(publication_inventory)?;
    publication_evidence.build.source_commit.clone_from(&commit);
    profile_internal_duration(
        "v1 evidence preparation",
        publication_evidence_started.elapsed(),
    );
    let normalization_started = Instant::now();
    let mut published = normalize_document_v1_with_evidence_best_effort_owned_at_inference(
        raw_document,
        publication_evidence,
        options.inference_level,
    )?;
    apply_inference_level(&mut published.document, options.inference_level);
    profile_internal_duration(
        "graph.json v1 normalization",
        normalization_started.elapsed(),
    );
    if published.document.nodes.is_empty() {
        return Err(CoreError::EmptyGraph);
    }
    let omissions = published.omissions;
    let report_health = current_orientation_health(options, omissions);
    // Legacy clustering and report code needs a compatibility projection, but
    // retaining it beside the complete typed authority doubles the dominant
    // graph working set. Move records into the projection and reconstruct the
    // strict authority after those consumers finish instead.
    let document = published.document.into_legacy_document()?;

    // A history realization must depend only on the target commit and build
    // profile. Prior community numbering is current-worktree operational state
    // and cannot influence the content-addressed result.
    let cluster_options = ClusterOptions {
        resolution: options.resolution,
        exclude_hubs_percentile: options.exclude_hubs,
    };
    let previous_started = Instant::now();
    let history_build = std::env::var_os("COMPASS_HISTORY_BUILD").is_some();
    let previous = if history_build {
        HashMap::new()
    } else {
        previous_communities(&output_dir.join("graph.json"))
    };
    let previous_elapsed = previous_started.elapsed();
    profile_internal_duration("load previous communities", previous_elapsed);
    let changed_sources = missing
        .iter()
        .map(|path| relative_fact_path(path, &root))
        .collect::<BTreeSet<_>>();
    let cluster_started = Instant::now();
    let clustered = cluster_incremental(
        &document,
        &previous,
        &changed_sources,
        cluster_options,
        IncrementalClusterLimits::default(),
    );
    let cluster_elapsed = cluster_started.elapsed();
    profile_internal_duration(
        if clustered.used_incremental {
            "bounded incremental clustering"
        } else {
            "Louvain clustering"
        },
        cluster_elapsed,
    );
    internal_started = Instant::now();
    let communities = clustered.communities;
    timings.graph_assembly += stage_started.elapsed();
    stage_started = Instant::now();
    let labels = label_communities_by_hub(&document, &communities);
    profile_internal("community labeling", &mut internal_started);

    let graph_analyses = ||
     -> Result<
        (
            bool,
            Duration,
            Option<Value>,
            Option<compass_output::AgentOrientation>,
        ),
        CoreError,
    > {
        let started = Instant::now();
        let analysis_compute_started = Instant::now();
        let (cohesion, (gods, surprises, questions)) = rayon::join(
            || score_communities(&document, &communities),
            || graph_insights(&document, &communities, &labels, 10, 5, 10),
        );
        profile_internal_duration(
            "graph analyses computation",
            analysis_compute_started.elapsed(),
        );
        let analysis_render_started = Instant::now();
        let tokens = semantic_tokens(semantic);
        let analysis = if options.purpose == BuildPurpose::Extract {
            json!({
                "communities": communities.iter().map(|(key, value)| (key.to_string(), value)).collect::<BTreeMap<_, _>>(),
                "cohesion": cohesion.iter().map(|(key, value)| (key.to_string(), value)).collect::<BTreeMap<_, _>>(),
                "gods": gods,
                "surprises": surprises,
                "tokens": {"input": tokens.0, "output": tokens.1},
            })
        } else {
            json!({
                "communities": communities.iter().map(|(key, value)| (key.to_string(), value)).collect::<BTreeMap<_, _>>(),
                "cohesion": cohesion.iter().map(|(key, value)| (key.to_string(), value)).collect::<BTreeMap<_, _>>(),
                "gods": gods,
                "surprises": surprises,
                "questions": questions,
            })
        };
        if options.purpose == BuildPurpose::Extract {
            write_json_atomic(output_dir.join("analysis.json"), &analysis, true)?;
        } else {
            let labels_json = serde_json::to_string_pretty(&labels).map_err(|source| {
                CoreError::SerializeExtraction {
                    path: output_dir.join("labels.json"),
                    source,
                }
            })?;
            write_text_atomic(output_dir.join("labels.json"), &format!("{labels_json}\n"))?;
        }
        let detection_summary = report_detection_summary(
            detection.total_files,
            detection.total_words,
            detection.warning.clone(),
        );
        let orientation = if options.purpose == BuildPurpose::Update {
            let report_root = report_root_label(&options.root);
            let mut report_options = ReportOptions::new(&report_root);
            report_options.built_at_commit = commit.as_deref();
            report_options.health = report_health.clone();
            Some(agent_orientation(
                &document,
                &communities,
                &cohesion,
                &labels,
                &gods,
                &surprises,
                &detection_summary,
                TokenCost::default(),
                Some(&questions),
                None,
                &report_options,
            ))
        } else {
            None
        };
        let html_written = if options.purpose == BuildPurpose::Update {
            let html_path = output_dir.join("graph.html");
            if options.no_viz {
                remove_if_exists(&html_path)?;
                false
            } else {
                let rendered = match write_html(
                    &document,
                    &communities,
                    &html_path,
                    &HtmlOptions {
                        community_labels: (!labels.is_empty()).then_some(&labels),
                        node_limit: None,
                        ..HtmlOptions::default()
                    },
                ) {
                    Ok(rendered) => rendered,
                    Err(OutputError::HtmlTooLarge { .. }) => None,
                    Err(error) => return Err(CoreError::Output(error)),
                };
                let html_written = rendered.is_some();
                if !html_written {
                    remove_if_exists(&html_path)?;
                }
                html_written
            }
        } else {
            false
        };
        profile_internal_duration(
            "graph analysis and report rendering",
            analysis_render_started.elapsed(),
        );
        Ok((
            html_written,
            started.elapsed(),
            retain_artifacts.then_some(analysis),
            orientation,
        ))
    };
    let overview_output = || -> Result<(Duration, Option<GraphViewModel>), CoreError> {
        let started = Instant::now();
        let model = if options.purpose == BuildPurpose::Update {
            write_text_atomic(
                output_dir.join("source-root.txt"),
                &options.root.to_string_lossy(),
            )?;
            graph_overview_model(&document, &communities, &labels, &output_dir)?
        } else {
            None
        };
        Ok((started.elapsed(), model))
    };
    let (analysis_result, overview_result) = rayon::join(graph_analyses, overview_output);
    let (html_written, analysis_elapsed, retained_analysis, orientation) = analysis_result?;
    let (overview_elapsed, overview_model) = overview_result?;
    profile_internal_duration(
        "parallel graph analyses and report publication",
        analysis_elapsed,
    );
    profile_internal_duration(
        "graph root marker and overview publication",
        overview_elapsed,
    );

    // Move the compatibility records back into the typed authority before
    // publication. The consuming conversion drains each record, so the two
    // complete representations never coexist.
    let mut published_document = V1GraphDocument::from_legacy_document(document)?;

    let publish_started = Instant::now();
    let graph_output_started = Instant::now();
    let mut output_profile_started = Instant::now();
    let node_communities = communities
        .iter()
        .flat_map(|(community, members)| {
            members
                .iter()
                .map(move |member| (member.as_str(), *community))
        })
        .collect::<HashMap<_, _>>();
    for node in &mut published_document.nodes {
        let Some(&community_index) = node_communities.get(node.id.as_str()) else {
            continue;
        };
        let community = u64::try_from(community_index)
            .map_err(|_| CoreError::InvalidBuildState("community ID exceeds u64".to_owned()))?;
        node.community = Some(CommunityMetadata {
            id: community,
            label: labels.get(&community_index).cloned(),
            score: None,
            color: None,
        });
    }
    let published_nodes = published_document.nodes.len();
    let published_edges = published_document.links.len();
    let serialization_started = Instant::now();
    let (store_metrics, graph_seal) = if options.graph_storage.publishes_store() {
        let (metrics, seal) =
            if let Some(previous) = load_graph_delta_base(&output_dir, &published_document) {
                publish_graph_and_store_delta(&output_dir, &previous, &published_document)?
            } else {
                publish_graph_and_store_from_canonical(&output_dir, &published_document)?
            };
        (Some(metrics), Some(seal))
    } else {
        let graph_path = output_dir.join("graph.json");
        let receipt = write_atomic_with_digest(&graph_path, |writer| {
            write_canonical_graph_json(&published_document, writer).map_err(|source| {
                compass_files::FileError::Io {
                    path: graph_path.clone(),
                    source,
                }
            })
        })?;
        (
            None,
            Some(ArtifactSeal {
                bytes: receipt.bytes,
                sha256: receipt.sha256,
            }),
        )
    };
    if let Some(metrics) = store_metrics {
        record_store_metrics(&mut timings, metrics);
    }
    if options.purpose == BuildPurpose::Update {
        write_prepared_graph_overview(overview_model, &output_dir)?;
    }
    if let Some(mut orientation) = orientation {
        let seal = graph_seal.as_ref().ok_or_else(|| {
            CoreError::InvalidBuildState(
                "graph artifact seal is unavailable for Agent Orientation".to_owned(),
            )
        })?;
        orientation.evidence_status.artifact_set_identity = Some(format!("sha256:{}", seal.sha256));
        let report = render_agent_report_markdown(&orientation, false)?;
        let orientation_json = render_orientation_json(&orientation)?;
        write_text_atomic(output_dir.join("GRAPH_REPORT.md"), &report)?;
        write_text_atomic(
            output_dir.join("orientation.json"),
            &format!("{orientation_json}\n"),
        )?;
    }
    let serialization_elapsed = serialization_started.elapsed();
    profile_internal_duration("graph.json v1 serialization", serialization_elapsed);
    profile_internal("graph.json v1 publication", &mut output_profile_started);
    let graph_output_elapsed = graph_output_started.elapsed();
    let retained_document = retain_artifacts.then_some(published_document);
    profile_internal_duration("graph publication", graph_output_elapsed);
    internal_started = Instant::now();
    timings.graph_assembly += stage_started.elapsed();

    let mut manifest = prior_manifest;
    let (manifest_result, seals_result) = rayon::join(
        || {
            save_build_manifest(
                &mut manifest,
                &detection.files,
                &manifest_path,
                &root,
                semantic,
            )
        },
        || {
            write_semantic_marker(&output_dir, semantic)?;
            save_output_stats(
                &output_dir,
                published_nodes,
                published_edges,
                communities.len(),
                true,
                omissions,
            )
        },
    );
    manifest_result?;
    seals_result?;
    write_ast_fact_digest_state(&output_dir, &current_fact_state)?;
    if program.is_none() {
        program = join_program_worker(program_handle.take(), &mut timings)?;
    }
    if let Some(program) = program.as_mut() {
        program.finish_pending_seal(&mut timings)?;
    }
    publish_build_state(
        options,
        &output_dir,
        &manifest_path,
        sources.len(),
        published_nodes,
        published_edges,
        communities.len(),
        omissions,
        program.as_ref(),
        graph_seal,
        store_metrics.is_some(),
        &mut timings,
    )?;
    profile_internal("Program output and build seals", &mut internal_started);
    let published_output_dir = commit_snapshot(
        guard,
        &output_container,
        options.graph_storage,
        true,
        !options.graph_storage.publishes_store(),
        true,
        &mut timings,
    )?;
    timings.publish = publish_started.elapsed();
    let result = BuildResult {
        root,
        output_dir: published_output_dir,
        detection,
        files_considered: sources.len(),
        files_extracted: missing.len(),
        files_cached: sources.len().saturating_sub(missing.len()),
        empty_files,
        nodes: published_nodes,
        edges: published_edges,
        communities: communities.len(),
        omitted_nodes: omissions.nodes,
        omitted_edges: omissions.edges,
        identity_collisions: omissions.identity_collisions,
        partial_graph: omissions.is_partial(),
        html_written,
        outputs_changed: true,
        program_modules: program_modules(program.as_ref()),
        program_summaries: program_summaries(program.as_ref()),
        program_syntax_analyzed: program
            .as_ref()
            .map_or(0, |program| program.syntax_analyzed),
        program_syntax_reused: program.as_ref().map_or(0, |program| program.syntax_reused),
        program_artifacts_loaded: program
            .as_ref()
            .map_or(0, |program| program.artifacts_loaded),
        program_artifacts_reused: program
            .as_ref()
            .map_or(0, |program| program.artifacts_reused),
        program_artifact_documents_analyzed: program
            .as_ref()
            .map_or(0, |program| program.artifact_documents_analyzed),
        program_artifact_documents_reused: program
            .as_ref()
            .map_or(0, |program| program.artifact_documents_reused),
        program_conflicts: program.as_ref().map_or(0, |program| program.conflicts),
        timings,
    };
    let retained = retained_document.map(|document| RetainedBuildArtifacts {
        document,
        program: program.and_then(|program| program.analysis),
        analysis: retained_analysis,
    });
    Ok((result, retained))
}

fn oversized_source_extraction(
    path: &Path,
    max_source_bytes: u64,
) -> Result<Extraction, compass_languages::ExtractError> {
    let metadata = fs::metadata(path).map_err(|source| compass_files::FileError::Io {
        path: path.to_path_buf(),
        source,
    })?;
    let mut extraction = Extraction::default();
    extraction.extensions.insert(
        EXTRACTION_QUALITY_EXTENSION.to_owned(),
        serde_json::Value::String(EXTRACTION_QUALITY_PARTIAL.to_owned()),
    );
    extraction.extensions.insert(
        EXTRACTION_QUALITY_REASON_EXTENSION.to_owned(),
        serde_json::Value::String(format!(
            "source is {} bytes, exceeding the configured {} byte extraction limit",
            metadata.len(),
            max_source_bytes
        )),
    );
    Ok(extraction)
}

type ProgramWriteHandle = JoinHandle<Result<(ArtifactSeal, Duration), CoreError>>;

struct ProgramBuildSummary {
    seal: Option<ArtifactSeal>,
    pending_seal: Option<ProgramWriteHandle>,
    modules: usize,
    summaries: usize,
    providers: usize,
    syntax_analyzed: usize,
    syntax_reused: usize,
    artifacts_loaded: usize,
    artifacts_reused: usize,
    artifact_documents_analyzed: usize,
    artifact_documents_reused: usize,
    conflicts: usize,
    compiler_projection: Option<compass_program::CompilerProjection>,
    analysis: Option<compass_analysis::AnalysisBundle>,
}

impl ProgramBuildSummary {
    fn from_program(program: ProgramBuild, retain_analysis: bool) -> Self {
        let seal = ArtifactSeal::from_bytes(&program.canonical_bytes);
        Self::from_program_with_seal(program, retain_analysis, seal)
    }

    fn from_program_with_seal(
        program: ProgramBuild,
        retain_analysis: bool,
        seal: ArtifactSeal,
    ) -> Self {
        let modules = program.analysis.program.modules.len();
        let summaries = program.analysis.summaries.len();
        let providers = program.analysis.program.providers.len();
        let analysis = retain_analysis.then_some(program.analysis);
        Self {
            seal: Some(seal),
            pending_seal: None,
            modules,
            summaries,
            providers,
            syntax_analyzed: program.syntax_analyzed,
            syntax_reused: program.syntax_reused,
            artifacts_loaded: program.artifacts_loaded,
            artifacts_reused: program.artifacts_reused,
            artifact_documents_analyzed: program.artifact_documents_analyzed,
            artifact_documents_reused: program.artifact_documents_reused,
            conflicts: program.conflicts,
            compiler_projection: Some(program.compiler_projection),
            analysis,
        }
    }

    fn finish_pending_seal(&mut self, timings: &mut BuildTimings) -> Result<(), CoreError> {
        let Some(handle) = self.pending_seal.take() else {
            return Ok(());
        };
        let (seal, elapsed) = handle
            .join()
            .map_err(|_| CoreError::WorkerPanic("Program artifact publication".to_owned()))??;
        self.seal = Some(seal);
        timings.program_analysis = timings.program_analysis.saturating_add(elapsed);
        Ok(())
    }
}

fn program_modules(program: Option<&ProgramBuildSummary>) -> usize {
    program.map_or(0, |program| program.modules)
}

fn program_summaries(program: Option<&ProgramBuildSummary>) -> usize {
    program.map_or(0, |program| program.summaries)
}

fn program_providers(program: Option<&ProgramBuildSummary>) -> usize {
    program.map_or(0, |program| program.providers)
}

fn build_profile(options: &BuildOptions) -> BuildProfile {
    BuildProfile {
        purpose: match options.purpose {
            BuildPurpose::Update => "update",
            BuildPurpose::Extract => "extract",
        }
        .to_owned(),
        no_cluster: options.no_cluster,
        no_viz: options.no_viz,
        resolution: options.resolution,
        exclude_hubs: options.exclude_hubs,
        code_only: options.code_only,
        program_analysis: options.program_analysis,
        graph_storage: match options.graph_storage {
            GraphStorage::Json => "json",
            GraphStorage::Sqlite => "sqlite",
        }
        .to_owned(),
        inference_level: options.inference_level.as_str().to_owned(),
        max_source_bytes: options.max_source_bytes,
    }
}

fn current_orientation_health(
    options: &BuildOptions,
    omissions: PublicationOmissions,
) -> OrientationHealth {
    let publication = if omissions.is_partial() {
        PublicationStatus::Partial
    } else {
        PublicationStatus::Complete
    };
    let mut profile = format!(
        "{}; cluster={}; code_only={}; program={}; storage={}",
        match options.purpose {
            BuildPurpose::Update => "update",
            BuildPurpose::Extract => "extract",
        },
        !options.no_cluster,
        options.code_only,
        options.program_analysis,
        match options.graph_storage {
            GraphStorage::Json => "json",
            GraphStorage::Sqlite => "sqlite",
        }
    );
    if options.inference_level != InferenceLevel::Max {
        profile.push_str(&format!("; inference={}", options.inference_level.as_str()));
    }
    let mut exclusions = options.scope.exclude.clone();
    exclusions.extend(options.extra_excludes.iter().cloned());
    exclusions.sort();
    exclusions.dedup();
    OrientationHealth {
        freshness: FreshnessStatus::Current,
        freshness_basis: FreshnessBasis::JustBuiltSelectedInputs,
        publication: Some(publication),
        omitted_nodes: Some(omissions.nodes),
        omitted_edges: Some(omissions.edges),
        identity_collisions: Some(omissions.identity_collisions),
        diagnostic_examples_omitted: Some(omissions.examples_omitted),
        build_profile: Some(profile),
        scope_includes: options.scope.include.clone(),
        configured_exclusions: exclusions,
        corpus_measurements_available: true,
        ..OrientationHealth::default()
    }
}

fn report_detection_summary(
    total_files: usize,
    total_words: u64,
    warning: Option<String>,
) -> DetectionSummary {
    DetectionSummary {
        total_files,
        total_words: usize::try_from(total_words).unwrap_or(usize::MAX),
        warning,
    }
}

#[allow(clippy::too_many_arguments)]
fn publish_build_state(
    options: &BuildOptions,
    output_dir: &Path,
    manifest_path: &Path,
    files: usize,
    nodes: usize,
    edges: usize,
    communities: usize,
    omissions: PublicationOmissions,
    program: Option<&ProgramBuildSummary>,
    graph_seal: Option<ArtifactSeal>,
    store_ready: bool,
    timings: &mut BuildTimings,
) -> Result<(), CoreError> {
    let publish_store = options.graph_storage.publishes_store();
    if publish_store && !store_ready {
        let metrics = ensure_store_snapshot(output_dir)?;
        record_store_metrics(timings, metrics);
    }
    let mut required = vec![output_dir.join(OUTPUT_STATS_FILE)];
    if publish_store {
        required.push(output_dir.join(STORE_REF_FILE_NAME));
    }
    match options.purpose {
        BuildPurpose::Update => {
            required.push(output_dir.join("source-root.txt"));
            if !options.no_cluster {
                required.extend([
                    output_dir.join(GRAPH_OVERVIEW_FILE),
                    output_dir.join("labels.json"),
                    output_dir.join("GRAPH_REPORT.md"),
                    output_dir.join("orientation.json"),
                ]);
            }
        }
        BuildPurpose::Extract if !options.no_cluster => {
            required.push(output_dir.join("analysis.json"));
        }
        BuildPurpose::Extract => {}
    }
    for optional in ["graph.html", SEMANTIC_MARKER_FILE] {
        let path = output_dir.join(optional);
        if path.is_file() {
            required.push(path);
        }
    }
    let state = BuildState::capture(
        output_dir,
        build_profile(options),
        manifest_path,
        graph_seal,
        program.and_then(|program| program.seal.clone()),
        &required,
        SavedStats {
            files,
            nodes,
            edges,
            communities,
            omitted_nodes: omissions.nodes,
            omitted_edges: omissions.edges,
            identity_collisions: omissions.identity_collisions,
            program_modules: program_modules(program),
            program_summaries: program_summaries(program),
            program_providers: program_providers(program),
            program_conflicts: program.map_or(0, |program| program.conflicts),
        },
    )?;
    state.save(output_dir)
}

fn commit_snapshot(
    guard: BuildGuard,
    output_container: &Path,
    graph_storage: GraphStorage,
    store_ready: bool,
    presealed_artifacts: bool,
    root_artifacts_changed: bool,
    timings: &mut BuildTimings,
) -> Result<PathBuf, CoreError> {
    let publish_store = graph_storage.publishes_store();
    if !publish_store {
        for suffix in ["", "-wal", "-shm"] {
            remove_if_exists(
                &guard
                    .staging_directory()
                    .join(format!("{STORE_FILE_NAME}{suffix}")),
            )?;
        }
        remove_if_exists(&guard.staging_directory().join(STORE_REF_FILE_NAME))?;
    }
    if publish_store && !store_ready {
        let metrics = ensure_store_snapshot(guard.staging_directory())?;
        record_store_metrics(timings, metrics);
    }
    let mut artifacts = vec!["graph.json", "manifest.json", BUILD_STATE_FILE];
    if publish_store {
        artifacts.push(STORE_REF_FILE_NAME);
    }
    if guard.staging_directory().join("program.json").is_file() {
        artifacts.push("program.json");
    }
    if publish_store {
        let gc = garbage_collect_shared_store(output_container, guard.staging_directory())?;
        timings.store_gc_deleted_entries = timings
            .store_gc_deleted_entries
            .saturating_add(gc.deleted_entries);
        timings.store_write_transactions = timings
            .store_write_transactions
            .saturating_add(gc.delete_transactions);
    }
    let commit_started = Instant::now();
    if presealed_artifacts && !publish_store {
        guard.commit_with_presealed_artifacts(&artifacts)?;
    } else {
        guard.commit_with_artifacts(&artifacts)?;
    }
    profile_internal_duration(
        "snapshot seal and pointer publication",
        commit_started.elapsed(),
    );
    let root_projection_started = Instant::now();
    BuildGuard::publish_root_artifacts(output_container, &ROOT_ARTIFACTS, root_artifacts_changed)?;
    profile_internal_duration(
        "root artifact projection",
        root_projection_started.elapsed(),
    );
    Ok(BuildGuard::resolve_current_snapshot_directory(
        output_container,
    )?)
}

fn garbage_collect_shared_store(
    output_container: &Path,
    staging_directory: &Path,
) -> Result<GraphSnapshotGcStats, CoreError> {
    let parse_reference = |path: &Path, bytes: &[u8]| -> Result<StoreRef, CoreError> {
        let reference = serde_json::from_slice::<StoreRef>(bytes).map_err(|error| {
            CoreError::InvalidBuildState(format!(
                "invalid retained store reference at {}: {error}",
                path.display()
            ))
        })?;
        reference.validate()?;
        Ok(reference)
    };
    let staging_reference_path = staging_directory.join(STORE_REF_FILE_NAME);
    let staging_reference_bytes = fs::read(&staging_reference_path).map_err(|error| {
        CoreError::InvalidBuildState(format!(
            "could not read staging store reference at {}: {error}",
            staging_reference_path.display()
        ))
    })?;
    let mut retained_references = vec![parse_reference(
        &staging_reference_path,
        &staging_reference_bytes,
    )?];
    if let Ok(active_directory) = BuildGuard::resolve_current_snapshot_directory(output_container) {
        let path = active_directory.join(STORE_REF_FILE_NAME);
        if let Ok(bytes) = fs::read(&path) {
            retained_references.push(parse_reference(&path, &bytes)?);
        }
    }
    retained_references.sort_by(|left, right| {
        left.snapshot_id
            .cmp(&right.snapshot_id)
            .then_with(|| left.manifest_digest.cmp(&right.manifest_digest))
    });
    retained_references.dedup_by(|left, right| {
        left.snapshot_id == right.snapshot_id && left.manifest_digest == right.manifest_digest
    });

    let mut all_references = retained_references.clone();
    for directory in BuildGuard::complete_snapshot_directories(output_container)? {
        let path = directory.join(STORE_REF_FILE_NAME);
        let Ok(bytes) = fs::read(&path) else {
            continue;
        };
        all_references.push(parse_reference(&path, &bytes)?);
    }
    all_references.sort_by(|left, right| {
        left.snapshot_id
            .cmp(&right.snapshot_id)
            .then_with(|| left.manifest_digest.cmp(&right.manifest_digest))
    });
    all_references.dedup_by(|left, right| {
        left.snapshot_id == right.snapshot_id && left.manifest_digest == right.manifest_digest
    });
    let selectors = retained_references
        .into_iter()
        .map(|reference| SnapshotSelector {
            schema: GRAPH_SNAPSHOT_SELECTOR_SCHEMA_V1.to_owned(),
            snapshot_id: reference.snapshot_id,
            manifest_digest: reference.manifest_digest,
        })
        .collect::<Vec<_>>();
    let graph_path = staging_directory.join("graph.json");
    let store = SqliteStore::open(local_sqlite_store_path(&graph_path))?;
    if all_references.len() <= 2 && !graph_snapshot_needs_gc(&store, 2)? {
        return Ok(GraphSnapshotGcStats::default());
    }
    let stats = garbage_collect_graph_snapshots(
        &store,
        &selectors,
        GRAPH_SNAPSHOT_MAX_OBJECTS.saturating_mul(8),
    )?;
    if stats.deleted_entries > 0 {
        store.reclaim_unused_pages(1_024)?;
    }
    store.checkpoint()?;
    Ok(stats)
}

/// Ensure every committed snapshot has a validated, queryable immutable
/// snapshot in the output root's shared store. The snapshot publishes only a
/// digest-bound selector beside the permanent `graph.json` artifact.
fn ensure_store_snapshot(output_dir: &Path) -> Result<StorePublishMetrics, CoreError> {
    let graph_path = output_dir.join("graph.json");
    let graph = V1GraphDocument::load(&graph_path)?;
    if graph.graph.schema != GRAPH_SCHEMA_V1 {
        return Err(CoreError::InvalidBuildState(format!(
            "graph.json has unsupported schema {}; expected {GRAPH_SCHEMA_V1}",
            graph.graph.schema
        )));
    }
    let store_path = local_sqlite_store_path(&graph_path);
    let store = SqliteStore::open(&store_path)?;
    let builder = GraphSnapshotBuilder::new();
    let prepared = builder.prepare_owned(&store, graph)?;
    finish_store_snapshot(output_dir, &store, &builder, prepared)
}

/// Return the previous graph only when the current publication can use the
/// bounded graph-delta path. The check is intentionally cheap and does not
/// serialize records; the snapshot layer repeats its validation before touching
/// the active store.
fn load_graph_delta_base(output_dir: &Path, current: &V1GraphDocument) -> Option<V1GraphDocument> {
    let previous = V1GraphDocument::load(&output_dir.join("graph.json")).ok()?;
    graph_delta_candidate(&previous, current).then_some(previous)
}

fn graph_delta_candidate(previous: &V1GraphDocument, current: &V1GraphDocument) -> bool {
    if previous.directed != current.directed || previous.multigraph != current.multigraph {
        return false;
    }
    // The normal publication path emits both collections in stable-ID order,
    // but graph.json is still an input boundary: an older, hand-authored, or
    // otherwise non-canonical yet structurally valid artifact can reach this
    // check. Never feed such records to the merge walk. Falling back to full
    // publication preserves correctness and restores canonical order on disk.
    if !records_are_sorted_by(&previous.nodes, |node| node.id.as_str())
        || !records_are_sorted_by(&previous.links, |edge| edge.id.as_str())
        || !records_are_sorted_by(&current.nodes, |node| node.id.as_str())
        || !records_are_sorted_by(&current.links, |edge| edge.id.as_str())
    {
        return false;
    }
    // V1 publication sorts both records by stable ID. A merge walk avoids
    // four BTreeMap allocations on every incremental build while preserving
    // the same changed-record count. The snapshot layer repeats its complete
    // validation before applying a delta, so this remains only a cheap
    // candidate check and never decides graph meaning.
    let changed_nodes =
        changed_record_count(&previous.nodes, &current.nodes, |node| node.id.as_str());
    let changed_edges =
        changed_record_count(&previous.links, &current.links, |edge| edge.id.as_str());
    let changed_records = changed_nodes.saturating_add(changed_edges);
    let total_records = previous
        .nodes
        .len()
        .saturating_add(previous.links.len())
        .max(1);
    if changed_records == 0 {
        return previous.graph != current.graph;
    }
    if changed_records > total_records.saturating_div(4).max(64) {
        return false;
    }
    true
}

fn records_are_sorted_by<T, F>(records: &[T], key: F) -> bool
where
    F: Fn(&T) -> &str,
{
    records
        .windows(2)
        .all(|records| key(&records[0]) <= key(&records[1]))
}

fn changed_record_count<T, F>(previous: &[T], current: &[T], key: F) -> usize
where
    T: PartialEq,
    F: Fn(&T) -> &str,
{
    debug_assert!(records_are_sorted_by(previous, &key));
    debug_assert!(records_are_sorted_by(current, &key));

    let mut previous_index = 0;
    let mut current_index = 0;
    let mut changed = 0;
    while previous_index < previous.len() || current_index < current.len() {
        match (previous.get(previous_index), current.get(current_index)) {
            (Some(previous_record), Some(current_record)) => {
                match key(previous_record).cmp(key(current_record)) {
                    std::cmp::Ordering::Less => {
                        changed += 1;
                        previous_index += 1;
                    }
                    std::cmp::Ordering::Greater => {
                        changed += 1;
                        current_index += 1;
                    }
                    std::cmp::Ordering::Equal => {
                        if previous_record != current_record {
                            changed += 1;
                        }
                        previous_index += 1;
                        current_index += 1;
                    }
                }
            }
            (Some(_), None) => {
                changed += previous.len().saturating_sub(previous_index);
                break;
            }
            (None, Some(_)) => {
                changed += current.len().saturating_sub(current_index);
                break;
            }
            (None, None) => break,
        }
    }
    changed
}

fn publish_graph_and_store_from_canonical(
    output_dir: &Path,
    graph: &V1GraphDocument,
) -> Result<(StorePublishMetrics, ArtifactSeal), CoreError> {
    if graph.graph.schema != GRAPH_SCHEMA_V1 {
        return Err(CoreError::InvalidBuildState(format!(
            "graph has unsupported schema {}; expected {GRAPH_SCHEMA_V1}",
            graph.graph.schema
        )));
    }
    let graph_path = output_dir.join("graph.json");
    let store = SqliteStore::open(local_sqlite_store_path(&graph_path))?;
    let builder = GraphSnapshotBuilder::new();
    let (graph_receipt, content) = rayon::join(
        || {
            write_atomic_with_digest(&graph_path, |writer| {
                write_canonical_graph_json(graph, writer).map_err(|source| {
                    compass_files::FileError::Io {
                        path: graph_path.clone(),
                        source,
                    }
                })
            })
        },
        || builder.prepare_content(&store, graph),
    );
    let graph_receipt = graph_receipt?;
    let graph_seal = ArtifactSeal {
        bytes: graph_receipt.bytes,
        sha256: graph_receipt.sha256.clone(),
    };
    let prepared =
        builder.finish_content(&store, content?, graph_receipt.sha256, graph_receipt.bytes)?;
    let metrics = finish_store_snapshot(output_dir, &store, &builder, prepared)?;
    Ok((metrics, graph_seal))
}

fn publish_graph_and_store_delta(
    output_dir: &Path,
    previous: &V1GraphDocument,
    graph: &V1GraphDocument,
) -> Result<(StorePublishMetrics, ArtifactSeal), CoreError> {
    if graph.graph.schema != GRAPH_SCHEMA_V1 {
        return Err(CoreError::InvalidBuildState(format!(
            "graph has unsupported schema {}; expected {GRAPH_SCHEMA_V1}",
            graph.graph.schema
        )));
    }
    let graph_path = output_dir.join("graph.json");
    let store = SqliteStore::open(local_sqlite_store_path(&graph_path))?;
    let builder = GraphSnapshotBuilder::new();
    let (graph_receipt, content) = rayon::join(
        || {
            write_atomic_with_digest(&graph_path, |writer| {
                write_canonical_graph_json(graph, writer).map_err(|source| {
                    compass_files::FileError::Io {
                        path: graph_path.clone(),
                        source,
                    }
                })
            })
        },
        || builder.prepare_graph_delta(&store, previous, graph),
    );
    let graph_receipt = graph_receipt?;
    let graph_seal = ArtifactSeal {
        bytes: graph_receipt.bytes,
        sha256: graph_receipt.sha256.clone(),
    };
    let content = content.or_else(|_| builder.prepare_content(&store, graph))?;
    let prepared =
        builder.finish_content(&store, content, graph_receipt.sha256, graph_receipt.bytes)?;
    let metrics = finish_store_snapshot(output_dir, &store, &builder, prepared)?;
    Ok((metrics, graph_seal))
}

fn finish_store_snapshot(
    output_dir: &Path,
    store: &SqliteStore,
    builder: &GraphSnapshotBuilder,
    prepared: compass_graph::PreparedGraphSnapshot,
) -> Result<StorePublishMetrics, CoreError> {
    let selector = builder.activate(store, &prepared)?;
    let reference =
        store.graph_snapshot_reference_for(&selector.snapshot_id, &selector.manifest_digest)?;
    write_store_ref(output_dir, &reference)?;
    store.record_retention_metadata(&reference, compass_store::MAX_SCAN_ITEMS)?;
    store.checkpoint()?;
    Ok(StorePublishMetrics {
        new_objects: prepared.new_objects,
        reused_objects: prepared.reused_objects,
        write_transactions: prepared.write_transactions.saturating_add(2),
        bytes_written: prepared.bytes_written,
    })
}

fn record_store_metrics(timings: &mut BuildTimings, metrics: StorePublishMetrics) {
    timings.store_new_objects = timings
        .store_new_objects
        .saturating_add(metrics.new_objects);
    timings.store_reused_objects = timings
        .store_reused_objects
        .saturating_add(metrics.reused_objects);
    timings.store_write_transactions = timings
        .store_write_transactions
        .saturating_add(metrics.write_transactions);
    timings.store_bytes_written = timings
        .store_bytes_written
        .saturating_add(metrics.bytes_written);
}

fn write_store_ref(output_dir: &Path, reference: &StoreRef) -> Result<(), CoreError> {
    compass_files::write_json_atomic(output_dir.join(STORE_REF_FILE_NAME), &reference, false)?;
    Ok(())
}

#[cfg(target_os = "macos")]
fn default_ast_workers(missing: usize) -> usize {
    available_worker_count()
        .min(default_ast_worker_cap(missing))
        .max(1)
}

#[cfg(not(target_os = "macos"))]
fn default_ast_workers(missing: usize) -> usize {
    available_worker_count()
        .min(default_ast_worker_cap(missing))
        .max(1)
}

const DEFAULT_AST_WORKER_CAP: usize = 8;
#[cfg(target_os = "macos")]
fn available_worker_count() -> usize {
    num_cpus::get().max(num_cpus::get_physical())
}

#[cfg(not(target_os = "macos"))]
fn available_worker_count() -> usize {
    num_cpus::get()
}

// Large cold repositories retain enough per-file parser state for worker
// parallelism to dominate peak RSS. Keep the automatic path bounded by the
// host-aware pipeline ceiling; callers that have a known memory budget can
// still opt into a different count through BuildOptions::max_workers.
const LARGE_REPOSITORY_AST_WORKER_CAP: usize = PIPELINE_RAYON_WORKER_CAP;
const LARGE_REPOSITORY_AST_WORKER_MIN_FILES: usize = 1_024;
const AUTOMATIC_PARALLEL_EXTRACT_MIN_FILES: usize = 32;

fn default_ast_worker_cap(missing: usize) -> usize {
    if missing < LARGE_REPOSITORY_AST_WORKER_MIN_FILES {
        DEFAULT_AST_WORKER_CAP
    } else {
        LARGE_REPOSITORY_AST_WORKER_CAP
    }
}

fn should_parallel_extract(options: &BuildOptions, missing: usize) -> bool {
    missing >= AUTOMATIC_PARALLEL_EXTRACT_MIN_FILES
        || options.max_workers.is_some_and(|workers| workers > 1)
}

fn is_php_source_path(path: &Path) -> bool {
    path.extension()
        .is_some_and(|extension| extension.eq_ignore_ascii_case("php"))
}

fn semantic_layer_is_empty(layer: &SemanticLayer) -> bool {
    layer.refreshed_files.is_empty()
        && layer.partial_files.is_empty()
        && ["nodes", "edges", "hyperedges"].into_iter().all(|key| {
            layer
                .fragment
                .get(key)
                .and_then(serde_json::Value::as_array)
                .is_none_or(Vec::is_empty)
        })
}

fn write_semantic_marker(
    output_dir: &Path,
    semantic: Option<&SemanticLayer>,
) -> Result<(), CoreError> {
    let (_, output_tokens) = semantic_tokens(semantic);
    if output_tokens > 0 || semantic.is_some_and(|layer| !semantic_layer_is_empty(layer)) {
        write_json_atomic(
            output_dir.join(SEMANTIC_MARKER_FILE),
            &json!({"output_tokens": output_tokens}),
            false,
        )?;
    }
    Ok(())
}

fn finalize_ast_extraction(extraction: &mut Extraction, root: &Path) {
    let mut external_id_remap = HashMap::new();
    let mut canonical_sources = HashMap::<String, PathBuf>::new();
    for node in &mut extraction.nodes {
        let Some(source) = node
            .attributes
            .get("source_file")
            .and_then(serde_json::Value::as_str)
            .map(str::to_owned)
        else {
            continue;
        };
        let source_path = Path::new(&source);
        if !source_path.is_absolute() {
            continue;
        }
        let canonical = canonical_sources
            .entry(source.clone())
            .or_insert_with(|| rooted_source_identity(source_path, root));
        if canonical.starts_with(root) {
            continue;
        }
        let portable = portable_out_of_root_source(source_path, root);
        if node.id == make_id(&[&source]) {
            external_id_remap.insert(node.id.clone(), make_id(&["ext", &portable]));
        }
        node.attributes.insert(
            "source_file".to_owned(),
            serde_json::Value::String(portable),
        );
    }
    if !external_id_remap.is_empty() {
        for node in &mut extraction.nodes {
            if let Some(canonical) = external_id_remap.get(&node.id) {
                node.id.clone_from(canonical);
            }
        }
        for edge in &mut extraction.edges {
            if let Some(canonical) = external_id_remap.get(&edge.source) {
                edge.source.clone_from(canonical);
            }
            if let Some(canonical) = external_id_remap.get(&edge.target) {
                edge.target.clone_from(canonical);
            }
        }
    }
    if extraction
        .nodes
        .len()
        .saturating_add(extraction.edges.len())
        < 100_000
    {
        extraction.nodes.iter_mut().for_each(|node| {
            normalize_source_attribute_cached(&mut node.attributes, root, &canonical_sources);
            node.attributes.remove("_callable");
            node.attributes
                .entry("_origin".to_owned())
                .or_insert_with(|| serde_json::Value::String("ast".to_owned()));
        });
        extraction.edges.iter_mut().for_each(|edge| {
            normalize_source_attribute_cached(&mut edge.attributes, root, &canonical_sources);
            edge.attributes
                .entry("_origin".to_owned())
                .or_insert_with(|| serde_json::Value::String("ast".to_owned()));
        });
    } else {
        rayon::join(
            || {
                extraction.nodes.par_iter_mut().for_each(|node| {
                    normalize_source_attribute_cached(
                        &mut node.attributes,
                        root,
                        &canonical_sources,
                    );
                    node.attributes.remove("_callable");
                    node.attributes
                        .entry("_origin".to_owned())
                        .or_insert_with(|| serde_json::Value::String("ast".to_owned()));
                });
            },
            || {
                extraction.edges.par_iter_mut().for_each(|edge| {
                    normalize_source_attribute_cached(
                        &mut edge.attributes,
                        root,
                        &canonical_sources,
                    );
                    edge.attributes
                        .entry("_origin".to_owned())
                        .or_insert_with(|| serde_json::Value::String("ast".to_owned()));
                });
            },
        );
    }
}

fn prepare_portable_ast_cache_entry(extraction: &mut Extraction, source: &Path, root: &Path) {
    let canonical = fs::canonicalize(source).unwrap_or_else(|_| source.to_path_buf());
    let Ok(relative) = source
        .strip_prefix(root)
        .or_else(|_| canonical.strip_prefix(root))
    else {
        return;
    };
    let portable = relative.to_string_lossy().replace('\\', "/");
    let mut normalized_paths = HashMap::<String, String>::new();
    let mut normalized_origin_paths = HashMap::<String, String>::new();
    let mut set_portable = |attributes: &mut serde_json::Map<String, serde_json::Value>| {
        for key in ["source_file", "origin_file"] {
            let Some(value) = attributes
                .get(key)
                .and_then(serde_json::Value::as_str)
                .filter(|value| !value.is_empty())
            else {
                continue;
            };
            let normalized = normalize_portable_origin_path(
                value,
                source,
                &canonical,
                &portable,
                root,
                &mut normalized_origin_paths,
                &mut normalized_paths,
            );
            if normalized != value {
                attributes.insert(key.to_owned(), serde_json::Value::String(normalized));
            }
        }
        if let Some(anchor) = attributes
            .get_mut("origin_source_anchor")
            .and_then(serde_json::Value::as_object_mut)
            && let Some(value) = anchor
                .get("file")
                .and_then(serde_json::Value::as_str)
                .map(str::to_owned)
        {
            anchor.insert(
                "file".to_owned(),
                serde_json::Value::String(normalize_portable_origin_path(
                    &value,
                    source,
                    &canonical,
                    &portable,
                    root,
                    &mut normalized_origin_paths,
                    &mut normalized_paths,
                )),
            );
        }
        if let Some(target) = attributes
            .get("target_file")
            .and_then(serde_json::Value::as_str)
            .map(str::to_owned)
        {
            let normalized = normalize_portable_path(&target, root, &mut normalized_paths);
            attributes.insert(
                "target_file".to_owned(),
                serde_json::Value::String(normalized),
            );
        }
    };
    for node in &mut extraction.nodes {
        set_portable(&mut node.attributes);
    }
    for edge in &mut extraction.edges {
        set_portable(&mut edge.attributes);
    }
    for hyperedge in &mut extraction.hyperedges {
        if let Some(attributes) = hyperedge.as_object_mut() {
            set_portable(attributes);
        }
    }
    for fact in &mut extraction.framework_facts {
        let source_file = match fact {
            RawFrameworkFact::Route(route) => &mut route.anchor.source_file,
            RawFrameworkFact::Domain(domain) => &mut domain.anchor.source_file,
            RawFrameworkFact::Annotation(annotation) => &mut annotation.anchor.source_file,
        };
        *source_file = normalize_portable_origin_path(
            source_file,
            source,
            &canonical,
            &portable,
            root,
            &mut normalized_origin_paths,
            &mut normalized_paths,
        );
    }
    if let Some(calls) = extraction.raw_calls.as_mut() {
        for call in calls {
            if !call.source_file.is_empty() {
                call.source_file.clone_from(&portable);
            }
        }
    }
    if let Some(evidence) = extraction.semantic_evidence.as_mut() {
        for range in evidence
            .declarations
            .iter_mut()
            .map(|fact| &mut fact.range)
            .chain(evidence.scopes.iter_mut().map(|fact| &mut fact.range))
            .chain(evidence.bindings.iter_mut().map(|fact| &mut fact.range))
            .chain(evidence.occurrences.iter_mut().map(|fact| &mut fact.range))
            .chain(
                evidence
                    .diagnostics
                    .iter_mut()
                    .filter_map(|diagnostic| diagnostic.range.as_mut()),
            )
        {
            if Path::new(&range.source_file) == source
                || Path::new(&range.source_file).is_absolute()
            {
                range.source_file.clone_from(&portable);
            }
        }
    }
}

fn normalize_portable_path(
    value: &str,
    root: &Path,
    cache: &mut HashMap<String, String>,
) -> String {
    if let Some(normalized) = cache.get(value) {
        return normalized.clone();
    }
    let path = Path::new(value);
    let canonical = if path.is_absolute() {
        canonicalize_allow_missing(path)
    } else {
        canonicalize_allow_missing(&root.join(path))
    };
    let normalized = canonical.strip_prefix(root).map_or_else(
        |_| portable_out_of_root_source(&canonical, root),
        |relative| relative.to_string_lossy().replace('\\', "/"),
    );
    cache.insert(value.to_owned(), normalized.clone());
    normalized
}

fn normalize_portable_origin_path(
    value: &str,
    source: &Path,
    canonical: &Path,
    portable: &str,
    root: &Path,
    origin_cache: &mut HashMap<String, String>,
    path_cache: &mut HashMap<String, String>,
) -> String {
    if let Some(normalized) = origin_cache.get(value) {
        return normalized.clone();
    }
    let path = Path::new(value);
    let normalized = if path == source {
        portable.to_owned()
    } else {
        let canonical_value = if path.is_absolute() {
            canonicalize_allow_missing(path)
        } else {
            canonicalize_allow_missing(&root.join(path))
        };
        if canonical_value == canonical {
            portable.to_owned()
        } else {
            normalize_portable_path(value, root, path_cache)
        }
    };
    origin_cache.insert(value.to_owned(), normalized.clone());
    normalized
}

fn normalize_source_attribute_cached(
    attributes: &mut serde_json::Map<String, serde_json::Value>,
    root: &Path,
    canonical_sources: &HashMap<String, PathBuf>,
) {
    let Some(source) = attributes
        .get("source_file")
        .and_then(serde_json::Value::as_str)
    else {
        return;
    };
    let path = Path::new(source);
    if !path.is_absolute() {
        return;
    }
    let canonical_fallback;
    let canonical_path = if let Some(canonical) = canonical_sources.get(source) {
        canonical
    } else {
        canonical_fallback = fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
        &canonical_fallback
    };
    let Ok(relative) = canonical_path.strip_prefix(root) else {
        return;
    };
    attributes.insert(
        "source_file".to_owned(),
        serde_json::Value::String(relative.to_string_lossy().replace('\\', "/")),
    );
}

fn portable_out_of_root_source(path: &Path, root: &Path) -> String {
    use std::path::Component;

    let path = canonicalize_allow_missing(path);
    let root = canonicalize_allow_missing(root);
    let path_components = path.components().collect::<Vec<_>>();
    let root_components = root.components().collect::<Vec<_>>();
    let common = path_components
        .iter()
        .zip(&root_components)
        .take_while(|(left, right)| left == right)
        .count();
    if common == 0
        || matches!(
            (path_components.first(), root_components.first()),
            (Some(Component::Prefix(left)), Some(Component::Prefix(right))) if left != right
        )
    {
        return path
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or_default()
            .to_owned();
    }
    let upward = root_components.len().saturating_sub(common);
    if upward > 3 {
        return path
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or_default()
            .to_owned();
    }
    let mut relative = PathBuf::new();
    for _ in 0..upward {
        relative.push("..");
    }
    for component in &path_components[common..] {
        relative.push(component.as_os_str());
    }
    relative.to_string_lossy().replace('\\', "/")
}

fn canonicalize_allow_missing(path: &Path) -> PathBuf {
    if let Ok(canonical) = fs::canonicalize(path) {
        return canonical;
    }
    let mut cursor = path;
    let mut suffix = Vec::new();
    while let Some(name) = cursor.file_name() {
        suffix.push(name.to_os_string());
        let Some(parent) = cursor.parent() else {
            break;
        };
        cursor = parent;
        if let Ok(mut canonical) = fs::canonicalize(cursor) {
            for component in suffix.iter().rev() {
                canonical.push(component);
            }
            return canonical;
        }
    }
    path.to_path_buf()
}

fn finalize_semantic_extraction(extraction: &mut Extraction, root: &Path) {
    for node in &mut extraction.nodes {
        normalize_source_attribute(&mut node.attributes, root);
        node.attributes.insert(
            "_origin".to_owned(),
            serde_json::Value::String("semantic".to_owned()),
        );
        node.attributes.insert(
            "extractor".to_owned(),
            serde_json::Value::String(SEMANTIC_LAYER_EXTRACTOR.to_owned()),
        );
    }
    for edge in &mut extraction.edges {
        normalize_source_attribute(&mut edge.attributes, root);
        edge.attributes.insert(
            "_origin".to_owned(),
            serde_json::Value::String("semantic".to_owned()),
        );
        edge.attributes.insert(
            "extractor".to_owned(),
            serde_json::Value::String(SEMANTIC_LAYER_EXTRACTOR.to_owned()),
        );
    }
    for hyperedge in &mut extraction.hyperedges {
        let Some(attributes) = hyperedge.as_object_mut() else {
            continue;
        };
        normalize_source_attribute(attributes, root);
        attributes.insert(
            "_origin".to_owned(),
            serde_json::Value::String("semantic".to_owned()),
        );
        attributes.insert(
            "extractor".to_owned(),
            serde_json::Value::String(SEMANTIC_LAYER_EXTRACTOR.to_owned()),
        );
    }
}

fn ast_source_identity_map(sources: &[PathBuf], root: &Path) -> AHashMap<String, String> {
    let mut aliases = AHashMap::with_capacity(sources.len().saturating_mul(2));
    for source in sources {
        let identity = if source.is_absolute()
            && source.starts_with(root)
            && fs::symlink_metadata(source).is_ok_and(|metadata| !metadata.file_type().is_symlink())
        {
            source.clone()
        } else {
            rooted_source_identity(source, root)
        };
        let relative = identity
            .strip_prefix(root)
            .or_else(|_| source.strip_prefix(root));
        let Ok(relative) = relative else {
            continue;
        };
        let portable = make_id(&[&file_stem(relative)]);
        for path in [source.as_path(), identity.as_path()] {
            let legacy = make_id(&[&path.to_string_lossy()]);
            if legacy != portable {
                aliases.insert(legacy, portable.clone());
            }
        }
    }
    aliases
}

fn ast_extractions_are_portable(extractions: &[Extraction], root: &Path) -> bool {
    let rooted_prefix = format!("{}_", make_id(&[&root.to_string_lossy()]));
    let is_portable = |extraction: &Extraction| {
        extraction.nodes.iter().all(|node| {
            !node.id.starts_with(&rooted_prefix)
                && node
                    .attributes
                    .get("source_file")
                    .and_then(serde_json::Value::as_str)
                    .is_none_or(|source| !Path::new(source).is_absolute())
        }) && extraction.edges.iter().all(|edge| {
            !edge.source.starts_with(&rooted_prefix) && !edge.target.starts_with(&rooted_prefix)
        }) && extraction
            .raw_calls
            .iter()
            .flatten()
            .all(|call| !call.caller_nid.starts_with(&rooted_prefix))
            && extraction
                .semantic_evidence
                .as_ref()
                .is_none_or(|evidence| {
                    evidence.declarations.iter().all(|fact| {
                        !fact.graph_node_id.starts_with(&rooted_prefix)
                            && !Path::new(&fact.range.source_file).is_absolute()
                    }) && evidence
                        .scopes
                        .iter()
                        .all(|fact| !Path::new(&fact.range.source_file).is_absolute())
                        && evidence
                            .bindings
                            .iter()
                            .all(|fact| !Path::new(&fact.range.source_file).is_absolute())
                        && evidence
                            .occurrences
                            .iter()
                            .all(|fact| !Path::new(&fact.range.source_file).is_absolute())
                })
    };
    if extractions.len() < 256 {
        extractions.iter().all(is_portable)
    } else {
        extractions.par_iter().all(is_portable)
    }
}

fn collect_ast_id_remap(
    extractions: &[Extraction],
    root: &Path,
    live_id_remap: &AHashMap<String, String>,
) -> AHashMap<String, String> {
    let node_ids = extractions
        .iter()
        .flat_map(|extraction| extraction.nodes.iter())
        .map(|node| node.id.as_str())
        .collect::<AHashSet<_>>();
    let root_marker = format!("{}_", make_id(&[&root.to_string_lossy()]));
    if extractions.len() < 256 {
        return collect_ast_id_remap_chunk(
            extractions,
            root,
            live_id_remap,
            &node_ids,
            &root_marker,
        );
    }
    let target_chunks = rayon::current_num_threads().saturating_mul(2).max(1);
    let chunk_size = extractions.len().div_ceil(target_chunks);
    let chunks = extractions
        .par_chunks(chunk_size)
        .map(|chunk| {
            collect_ast_id_remap_chunk(chunk, root, live_id_remap, &node_ids, &root_marker)
        })
        .collect::<Vec<_>>();
    let capacity = chunks.iter().map(|chunk| chunk.len()).sum();
    let mut remap = AHashMap::with_capacity(capacity);
    // Indexed parallel collection preserves source chunk order, so a duplicate
    // ID keeps the same last-extraction-wins behavior as the sequential pass.
    for chunk in chunks {
        remap.extend(chunk);
    }
    remap
}

fn collect_ast_id_remap_chunk(
    extractions: &[Extraction],
    root: &Path,
    live_id_remap: &AHashMap<String, String>,
    node_ids: &AHashSet<&str>,
    root_marker: &str,
) -> AHashMap<String, String> {
    // Python first rewrites every exact ID derived from a file that is really
    // present in the detected corpus. This also catches references emitted by
    // another extractor (for example an .lpk unit pointing at sample.pas), but
    // deliberately leaves IDs for absent project references untouched.
    let node_count = extractions
        .iter()
        .map(|extraction| extraction.nodes.len())
        .sum();
    let mut id_remap = AHashMap::with_capacity(node_count);
    let mut source_identities = HashMap::<String, (PathBuf, String, String)>::new();
    let mut remap_rooted_id = |id: &str| {
        if let Some(portable) = live_id_remap.get(id) {
            id_remap.insert(id.to_owned(), portable.clone());
            return;
        }
        // A rooted endpoint without a node is an unresolved file reference.
        // Root-prefixed IDs that do have nodes may instead be symbols whose
        // file-derived prefix happens to match the checkout path.
        if node_ids.contains(id) {
            return;
        }
        if let Some(relative) = id.strip_prefix(root_marker)
            && !relative.is_empty()
        {
            id_remap
                .entry(id.to_owned())
                .or_insert_with(|| relative.to_owned());
        }
    };
    for extraction in extractions {
        for edge in &extraction.edges {
            remap_rooted_id(&edge.source);
            remap_rooted_id(&edge.target);
        }
    }
    for node in extractions
        .iter()
        .flat_map(|extraction| extraction.nodes.iter())
    {
        // An exact alias for a detected file is stronger than a symbol-prefix
        // interpretation from the referring node's source file.
        if let Some(portable) = live_id_remap.get(&node.id) {
            id_remap.insert(node.id.clone(), portable.clone());
            continue;
        }
        let Some(source) = node
            .attributes
            .get("source_file")
            .and_then(serde_json::Value::as_str)
        else {
            continue;
        };
        let (absolute, old_prefix, new_prefix) = source_identities
            .entry(source.to_owned())
            .or_insert_with(|| {
                let source_path = Path::new(source);
                let old_prefix = make_id(&[&file_stem(source_path)]);
                if source_path.is_relative() {
                    return (root.join(source_path), old_prefix.clone(), old_prefix);
                }
                let absolute = rooted_source_identity(source_path, root);
                let new_prefix = absolute.strip_prefix(root).map_or_else(
                    |_| String::new(),
                    |relative| make_id(&[&file_stem(relative)]),
                );
                (absolute, old_prefix, new_prefix)
            });
        if absolute.strip_prefix(root).is_err() {
            continue;
        }
        if new_prefix.is_empty() {
            continue;
        }
        if old_prefix == new_prefix {
            continue;
        }
        if node
            .attributes
            .get("type")
            .and_then(serde_json::Value::as_str)
            == Some("package")
        {
            continue;
        }
        if node.id == make_id(&[source]) {
            id_remap.insert(node.id.clone(), new_prefix.clone());
        } else if node.id == *old_prefix
            && node
                .attributes
                .get("symbol_kind")
                .and_then(serde_json::Value::as_str)
                .is_some_and(|kind| kind != "file")
        {
            // Permissive grammars can surface punctuation-only constructs
            // such as a TypeScript `[]` type as symbols. Their normalized
            // label is empty, so the extractor falls back to the absolute
            // file-stem ID. Keep the symbol distinct from the file node while
            // replacing that checkout-root identity with a stable location.
            let label = node
                .attributes
                .get("label")
                .and_then(serde_json::Value::as_str)
                .map(|value| make_id(&[value]))
                .filter(|value| !value.is_empty())
                .unwrap_or_else(|| "symbol".to_owned());
            let line = node
                .attributes
                .get("line_start")
                .and_then(serde_json::Value::as_u64)
                .or_else(|| {
                    node.attributes
                        .get("source_location")
                        .and_then(serde_json::Value::as_str)
                        .and_then(|value| value.strip_prefix('L'))
                        .and_then(|value| value.parse::<u64>().ok())
                })
                .unwrap_or_default();
            id_remap.insert(node.id.clone(), format!("{new_prefix}_{label}_{line}"));
        } else if let Some(suffix) = node.id.strip_prefix(&format!("{old_prefix}_")) {
            id_remap.insert(node.id.clone(), format!("{new_prefix}_{suffix}"));
        }
    }
    id_remap
}

fn apply_ast_id(id: &mut String, id_remap: &AHashMap<String, String>, root_marker: &str) {
    if let Some(canonical) = id_remap.get(id) {
        if id
            .strip_prefix(root_marker)
            .is_some_and(|relative| relative == canonical)
        {
            id.drain(..root_marker.len());
        } else {
            id.clone_from(canonical);
        }
    }
}

fn apply_ast_id_remap(
    extraction: &mut Extraction,
    id_remap: &AHashMap<String, String>,
    root_marker: &str,
) {
    for node in &mut extraction.nodes {
        apply_ast_id(&mut node.id, id_remap, root_marker);
    }
    for edge in &mut extraction.edges {
        apply_ast_id(&mut edge.source, id_remap, root_marker);
        apply_ast_id(&mut edge.target, id_remap, root_marker);
    }
    if let Some(calls) = extraction.raw_calls.as_mut() {
        for call in calls {
            apply_ast_id(&mut call.caller_nid, id_remap, root_marker);
        }
    }
    if let Some(evidence) = extraction.semantic_evidence.as_mut() {
        for declaration in &mut evidence.declarations {
            apply_ast_id(&mut declaration.graph_node_id, id_remap, root_marker);
        }
    }
}

fn rooted_source_identity(path: &Path, root: &Path) -> PathBuf {
    if path.is_absolute() {
        canonical_identity(path)
    } else {
        canonical_identity(&root.join(path))
    }
}

fn normalize_source_attribute(
    attributes: &mut serde_json::Map<String, serde_json::Value>,
    root: &Path,
) {
    let Some(source) = attributes
        .get("source_file")
        .and_then(serde_json::Value::as_str)
    else {
        return;
    };
    let path = Path::new(source);
    if !path.is_absolute() {
        return;
    }
    let canonical_path = fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
    let Ok(relative) = canonical_path.strip_prefix(root) else {
        return;
    };
    attributes.insert(
        "source_file".to_owned(),
        serde_json::Value::String(relative.to_string_lossy().replace('\\', "/")),
    );
}

fn preserve_semantic_layer(
    extraction: &mut Extraction,
    graph_path: &Path,
    root: &Path,
    refreshed: &HashSet<PathBuf>,
) {
    let Ok(existing) = V1GraphDocument::load(graph_path) else {
        return;
    };
    let mut existing_raw = extraction_from_v1(&existing);
    let current_ast_by_key = extraction
        .nodes
        .iter()
        .filter_map(|node| raw_node_match_key(node).map(|key| (key, node.id.clone())))
        .collect::<HashMap<_, _>>();
    let typed_ast_remap = existing
        .nodes
        .iter()
        .filter(|node| node_has_origin(node, EvidenceOrigin::Ast))
        .filter_map(|node| {
            typed_node_match_key(node)
                .and_then(|key| current_ast_by_key.get(&key))
                .map(|current| (node.id.clone(), current.clone()))
        })
        .collect::<HashMap<_, _>>();
    let mut dropped_edges = HashSet::new();
    let mut remapped_edges = HashSet::new();
    let mut remap_diagnostics = Vec::new();
    let mut dropped_without_site = 0_usize;
    for (index, edge) in existing_raw.edges.iter_mut().enumerate() {
        let source = typed_ast_remap
            .get(&edge.source)
            .cloned()
            .unwrap_or_else(|| edge.source.clone());
        let target = typed_ast_remap
            .get(&edge.target)
            .cloned()
            .unwrap_or_else(|| edge.target.clone());
        if source == edge.source && target == edge.target {
            continue;
        }
        if !has_exact_remap_site(&edge.attributes) {
            dropped_edges.insert(index);
            dropped_without_site = dropped_without_site.saturating_add(1);
            if remap_diagnostics.len() < MAX_INCREMENTAL_REMAP_DIAGNOSTICS {
                let mut related_ids = edge
                    .attributes
                    .get(compass_model::provenance::TRUSTED_EDGE_RECORD_ATTRIBUTE)
                    .and_then(|record| record.get("id"))
                    .and_then(serde_json::Value::as_str)
                    .map(str::to_owned)
                    .into_iter()
                    .collect::<Vec<_>>();
                related_ids.extend([edge.source.clone(), edge.target.clone()]);
                remap_diagnostics.push(GraphDiagnostic {
                    severity: DiagnosticSeverity::Warning,
                    code: INCREMENTAL_REMAP_DROP_DIAGNOSTIC.to_owned(),
                    message: "dropped preserved edge whose AST endpoint changed without an authoritative producer wiring site".to_owned(),
                    anchor: None,
                    related_ids,
                });
            }
            continue;
        }
        edge.source = source;
        edge.target = target;
        append_endpoint_rewrite_evidence(
            &mut edge.attributes,
            EndpointRewriteEvidence {
                rule: EndpointRewriteRule::IncrementalAstEndpointRemap,
                score: 1.0,
            },
        );
        remapped_edges.insert(index);
    }
    if dropped_without_site > MAX_INCREMENTAL_REMAP_DIAGNOSTICS {
        remap_diagnostics.push(GraphDiagnostic {
            severity: DiagnosticSeverity::Warning,
            code: INCREMENTAL_REMAP_TRUNCATION_DIAGNOSTIC.to_owned(),
            message: format!(
                "omitted {} additional incremental remap diagnostics",
                dropped_without_site - MAX_INCREMENTAL_REMAP_DIAGNOSTICS
            ),
            anchor: None,
            related_ids: Vec::new(),
        });
    }
    let mut fresh_edge_sites = HashMap::<IncrementalEdgeKey, Vec<SourceAnchor>>::new();
    let raw_edge_sites = canonical_raw_edge_sites(&extraction.edges, root);
    for (edge, site) in extraction.edges.iter().zip(raw_edge_sites) {
        let Some(key) = incremental_edge_key(edge) else {
            continue;
        };
        let Some(site) = site else {
            continue;
        };
        fresh_edge_sites.entry(key).or_default().push(site);
    }
    for sites in fresh_edge_sites.values_mut() {
        sites.sort_by(|left, right| {
            incremental_anchor_key(left).cmp(&incremental_anchor_key(right))
        });
    }
    let mut mixed_edges = HashMap::<IncrementalEdgeKey, Vec<(usize, SourceAnchor)>>::new();
    for (index, (raw, typed)) in existing_raw
        .edges
        .iter()
        .zip(existing.links.iter())
        .enumerate()
    {
        if dropped_edges.contains(&index)
            || !typed
                .evidence
                .iter()
                .any(|evidence| evidence.origin == EvidenceOrigin::Ast)
            || !typed.evidence.iter().any(is_semantic_layer_evidence)
        {
            continue;
        }
        let Some(key) = incremental_edge_key(raw) else {
            continue;
        };
        let Some(site) = typed.relationship_site.clone() else {
            continue;
        };
        mixed_edges.entry(key).or_default().push((index, site));
    }
    let mut refreshed_mixed_sites = HashMap::<usize, SourceAnchor>::new();
    let mut semantic_only_mixed_edges = HashSet::<usize>::new();
    for (key, mut prior) in mixed_edges {
        let Some(current) = fresh_edge_sites.get(&key) else {
            semantic_only_mixed_edges.extend(prior.into_iter().map(|(index, _)| index));
            continue;
        };
        prior.sort_by(|left, right| {
            incremental_anchor_key(&left.1)
                .cmp(&incremental_anchor_key(&right.1))
                .then_with(|| left.0.cmp(&right.0))
        });
        // Exact shared sites are authoritative even when occurrence cardinality changed.
        // Equal residual cardinality has one stable sorted bijection (the existing moved-site
        // behavior). Unequal residual cardinality is ambiguous: fresh occurrences stay AST-only,
        // while prior occurrences may survive only through semantic-only revalidation below.
        let mut matched_current = vec![false; current.len()];
        let mut unmatched_prior = Vec::new();
        for (index, prior_site) in prior {
            let exact = current
                .iter()
                .enumerate()
                .position(|(current_index, site)| {
                    !matched_current[current_index]
                        && incremental_anchor_key(&prior_site) == incremental_anchor_key(site)
                });
            if let Some(current_index) = exact {
                matched_current[current_index] = true;
                refreshed_mixed_sites.insert(index, current[current_index].clone());
            } else {
                unmatched_prior.push((index, prior_site));
            }
        }
        let unmatched_current = current
            .iter()
            .enumerate()
            .filter(|(index, _)| !matched_current[*index])
            .map(|(_, site)| site)
            .collect::<Vec<_>>();
        if unmatched_prior.len() == unmatched_current.len() {
            for ((index, _), site) in unmatched_prior.into_iter().zip(unmatched_current) {
                refreshed_mixed_sites.insert(index, site.clone());
            }
        } else {
            semantic_only_mixed_edges.extend(unmatched_prior.into_iter().map(|(index, _)| index));
        }
    }
    let prior_diagnostics = existing_raw.extensions.remove(GRAPH_DIAGNOSTICS_EXTENSION);
    append_graph_diagnostics(extraction, prior_diagnostics, remap_diagnostics);
    for prior in &existing.nodes {
        let Some(current_raw_id) = typed_ast_remap.get(&prior.id) else {
            continue;
        };
        let Some(current) = extraction
            .nodes
            .iter_mut()
            .find(|node| node.id == *current_raw_id)
        else {
            continue;
        };
        let fresh_site = raw_source_anchor(&current.attributes, root);
        let semantic_evidence = prior
            .evidence
            .iter()
            .filter(|evidence| is_semantic_layer_evidence(evidence))
            .cloned()
            .map(|mut evidence| {
                rebind_preserved_node_evidence(
                    &mut evidence,
                    prior.source.as_ref(),
                    fresh_site.as_ref(),
                );
                evidence
            })
            .filter_map(|evidence| serde_json::to_value(evidence).ok())
            .collect::<Vec<_>>();
        if semantic_evidence.is_empty() {
            continue;
        }
        let evidence = current
            .attributes
            .entry(COALESCED_NODE_EVIDENCE_ATTRIBUTE.to_owned())
            .or_insert_with(|| serde_json::Value::Array(Vec::new()));
        let Some(evidence) = evidence.as_array_mut() else {
            continue;
        };
        evidence.extend(semantic_evidence);
        evidence.sort_by_cached_key(serde_json::Value::to_string);
        evidence.dedup();
    }
    let preserved_node_ids = existing
        .nodes
        .iter()
        .filter(|node| {
            !typed_ast_remap.contains_key(&node.id)
                && node.evidence.iter().any(is_semantic_layer_evidence)
        })
        .map(|node| node.id.as_str())
        .collect::<HashSet<_>>();
    let mut preserved_nodes = existing_raw
        .nodes
        .into_iter()
        .filter(|node| {
            preserved_node_ids.contains(node.id.as_str())
                && !source_in_set(node.attributes.get("source_file"), root, refreshed)
                && !source_was_deleted(node.attributes.get("source_file"), root)
        })
        .collect::<Vec<_>>();
    for node in &mut preserved_nodes {
        strip_preserved_ast_node_evidence(&mut node.attributes);
    }
    let all_ids = extraction
        .nodes
        .iter()
        .map(|node| node.id.clone())
        .chain(preserved_nodes.iter().map(|node| node.id.clone()))
        .collect::<std::collections::HashSet<_>>();
    let mut preserved_edges = existing_raw
        .edges
        .into_iter()
        .zip(existing.links)
        .enumerate()
        .filter_map(|(index, (mut raw, typed))| {
            if dropped_edges.contains(&index) {
                return None;
            }
            let preserve = typed.evidence.iter().any(is_semantic_layer_evidence)
                && all_ids.contains(&raw.source)
                && all_ids.contains(&raw.target)
                && !source_in_set(raw.attributes.get("source_file"), root, refreshed)
                && !source_was_deleted(raw.attributes.get("source_file"), root);
            if !preserve {
                return None;
            }
            if let Some(site) = refreshed_mixed_sites.get(&index) {
                refresh_preserved_mixed_edge(&mut raw.attributes, &raw.source, &raw.target, site);
            } else if semantic_only_mixed_edges.contains(&index)
                && !retain_preserved_mixed_edge_as_semantic_only(
                    &mut raw.attributes,
                    remapped_edges.contains(&index),
                )
            {
                return None;
            }
            Some(raw)
        })
        .collect::<Vec<_>>();
    extraction
        .nodes
        .extend(preserved_nodes.drain(..).map(|node| RawNodeRecord {
            id: node.id,
            attributes: node.attributes,
        }));
    extraction
        .edges
        .extend(preserved_edges.drain(..).map(|edge| RawEdgeRecord {
            source: edge.source,
            target: edge.target,
            attributes: edge.attributes,
        }));
}

type IncrementalEdgeKey = (String, String, String, Option<String>);

fn incremental_anchor_key(anchor: &SourceAnchor) -> (&str, u64, u64) {
    (&anchor.file, anchor.start_byte, anchor.end_byte)
}

fn incremental_edge_key(edge: &RawEdgeRecord) -> Option<IncrementalEdgeKey> {
    let relation = edge.attributes.get("relation")?.as_str()?;
    Some((
        edge.source.clone(),
        edge.target.clone(),
        canonical_edge_kind(relation)?.as_str().to_owned(),
        edge.attributes
            .get(OCCURRENCE_RULE_ATTRIBUTE)
            .and_then(serde_json::Value::as_str)
            .map(str::to_owned),
    ))
}

fn refresh_preserved_mixed_edge(
    attributes: &mut serde_json::Map<String, serde_json::Value>,
    source: &str,
    target: &str,
    fresh_site: &SourceAnchor,
) {
    let Some(record) = attributes.get_mut(TRUSTED_EDGE_RECORD_ATTRIBUTE) else {
        return;
    };
    let Ok(mut edge) =
        serde_json::from_value::<compass_model::code_graph::EdgeRecord>(record.clone())
    else {
        return;
    };
    let prior_site = edge.relationship_site.clone();
    edge.evidence.retain(|evidence| {
        is_semantic_layer_evidence(evidence)
            && evidence.rule.as_deref()
                != Some(EndpointRewriteRule::IncrementalAstEndpointRemap.as_str())
    });
    for evidence in &mut edge.evidence {
        if evidence.wiring_site.as_ref() == prior_site.as_ref() {
            evidence.wiring_site = Some(fresh_site.clone());
        }
        for anchor in &mut evidence.anchors {
            if Some(&*anchor) == prior_site.as_ref() {
                anchor.clone_from(fresh_site);
            }
        }
    }
    edge.source = source.to_owned();
    edge.target = target.to_owned();
    edge.relationship_site = Some(fresh_site.clone());
    *record = serde_json::to_value(edge).unwrap_or(serde_json::Value::Null);
    rebind_raw_anchor(attributes, fresh_site);
}

fn retain_preserved_mixed_edge_as_semantic_only(
    attributes: &mut serde_json::Map<String, serde_json::Value>,
    remapped_in_current_pass: bool,
) -> bool {
    if remapped_in_current_pass
        && !attributes
            .get("_endpoint_rewrite_rules")
            .and_then(serde_json::Value::as_array)
            .is_some_and(|rewrites| {
                rewrites.iter().any(|rewrite| {
                    rewrite.get("rule").and_then(serde_json::Value::as_str)
                        == Some(EndpointRewriteRule::IncrementalAstEndpointRemap.as_str())
                })
            })
    {
        return false;
    }
    let Some(record) = attributes.get_mut(TRUSTED_EDGE_RECORD_ATTRIBUTE) else {
        return false;
    };
    let Ok(mut edge) =
        serde_json::from_value::<compass_model::code_graph::EdgeRecord>(record.clone())
    else {
        return false;
    };
    edge.evidence.retain(|evidence| {
        is_semantic_layer_evidence(evidence)
            && evidence.rule.as_deref()
                != Some(EndpointRewriteRule::IncrementalAstEndpointRemap.as_str())
    });
    let Some(primary) = edge
        .evidence
        .iter()
        .find(|evidence| {
            evidence
                .rule
                .as_deref()
                .and_then(EndpointRewriteRule::from_wire_name)
                .is_none()
        })
        .cloned()
    else {
        // Endpoint rewrites describe transport, not an independently justified relationship.
        // Without a non-AST producer assertion, an unmatched prior occurrence is stale.
        return false;
    };
    edge.relationship_site = None;
    *record = serde_json::to_value(edge).unwrap_or(serde_json::Value::Null);
    for key in [
        "source_file",
        "source_location",
        "source_anchor",
        "start_byte",
        "end_byte",
        "line_start",
        "line_end",
        "column_start",
        "column_end",
        "_coalesced_edge_evidence",
    ] {
        attributes.remove(key);
    }
    if !remapped_in_current_pass {
        attributes.remove("_endpoint_rewrite_rules");
        attributes.remove(CONSUME_INCREMENTAL_ENDPOINT_REMAP_ATTRIBUTE);
    } else {
        attributes.insert(
            CONSUME_INCREMENTAL_ENDPOINT_REMAP_ATTRIBUTE.to_owned(),
            serde_json::Value::Bool(true),
        );
    }
    project_preserved_semantic_evidence(attributes, &primary);
    true
}

fn project_preserved_semantic_evidence(
    attributes: &mut serde_json::Map<String, serde_json::Value>,
    evidence: &compass_model::provenance::Provenance,
) {
    attributes.insert(
        "_origin".to_owned(),
        serde_json::Value::String(evidence.origin.as_str().to_owned()),
    );
    attributes.insert(
        "confidence".to_owned(),
        serde_json::Value::String(evidence.confidence.legacy_str().to_owned()),
    );
    attributes.insert(
        "extractor".to_owned(),
        serde_json::Value::String(evidence.extractor.clone()),
    );
    if let Some(rule) = &evidence.rule {
        attributes.insert("rule".to_owned(), serde_json::Value::String(rule.clone()));
    } else {
        attributes.remove("rule");
    }
    if let Some(score) = evidence.score {
        attributes.insert("confidence_score".to_owned(), score.into());
    } else {
        attributes.remove("confidence_score");
    }
    if evidence.candidates.is_empty() {
        attributes.remove("candidates");
    } else {
        attributes.insert(
            "candidates".to_owned(),
            serde_json::to_value(&evidence.candidates).unwrap_or(serde_json::Value::Null),
        );
    }
    let semantic_site = evidence
        .wiring_site
        .as_ref()
        .or_else(|| evidence.anchors.first());
    if let Some(site) = semantic_site {
        attributes.insert(
            "source_file".to_owned(),
            serde_json::Value::String(site.file.clone()),
        );
        attributes.insert(
            "source_anchor".to_owned(),
            serde_json::to_value(site).unwrap_or(serde_json::Value::Null),
        );
    }
}

fn rebind_raw_anchor(
    attributes: &mut serde_json::Map<String, serde_json::Value>,
    fresh_site: &SourceAnchor,
) {
    attributes.insert(
        "source_file".to_owned(),
        serde_json::Value::String(fresh_site.file.clone()),
    );
    attributes.insert(
        "source_location".to_owned(),
        serde_json::Value::String(format!(
            "L{}:{}-L{}:{}",
            fresh_site.start_line,
            fresh_site.start_column,
            fresh_site.end_line,
            fresh_site.end_column
        )),
    );
    attributes.insert(
        "source_anchor".to_owned(),
        serde_json::to_value(fresh_site).unwrap_or(serde_json::Value::Null),
    );
    for (key, value) in [
        ("start_byte", fresh_site.start_byte.into()),
        ("end_byte", fresh_site.end_byte.into()),
        ("line_start", fresh_site.start_line.into()),
        ("line_end", fresh_site.end_line.into()),
        ("column_start", fresh_site.start_column.into()),
        ("column_end", fresh_site.end_column.into()),
    ] {
        if attributes.contains_key(key) {
            attributes.insert(key.to_owned(), value);
        }
    }
}

fn strip_preserved_ast_node_evidence(attributes: &mut serde_json::Map<String, serde_json::Value>) {
    let Some(record) = attributes.get_mut(compass_model::provenance::TRUSTED_NODE_RECORD_ATTRIBUTE)
    else {
        return;
    };
    let Ok(mut node) =
        serde_json::from_value::<compass_model::code_graph::NodeRecord>(record.clone())
    else {
        return;
    };
    node.evidence.retain(is_semantic_layer_evidence);
    *record = serde_json::to_value(node).unwrap_or(serde_json::Value::Null);
}

fn rebind_preserved_node_evidence(
    evidence: &mut compass_model::provenance::Provenance,
    prior_site: Option<&SourceAnchor>,
    fresh_site: Option<&SourceAnchor>,
) {
    let (Some(prior_site), Some(fresh_site)) = (prior_site, fresh_site) else {
        return;
    };
    if evidence.wiring_site.as_ref() == Some(prior_site) {
        evidence.wiring_site = Some(fresh_site.clone());
    }
    for anchor in &mut evidence.anchors {
        if anchor == prior_site {
            anchor.clone_from(fresh_site);
        }
    }
}

fn raw_source_anchor(
    attributes: &serde_json::Map<String, serde_json::Value>,
    root: &Path,
) -> Option<SourceAnchor> {
    let mut site = attributes
        .get("source_anchor")
        .and_then(|value| serde_json::from_value::<SourceAnchor>(value.clone()).ok())
        .or_else(|| {
            Some(SourceAnchor {
                file: attributes.get("source_file")?.as_str()?.to_owned(),
                start_byte: attributes.get("start_byte")?.as_u64()?,
                end_byte: attributes.get("end_byte")?.as_u64()?,
                start_line: u32::try_from(attributes.get("line_start")?.as_u64()?).ok()?,
                start_column: u32::try_from(attributes.get("column_start")?.as_u64()?).ok()?,
                end_line: u32::try_from(attributes.get("line_end")?.as_u64()?).ok()?,
                end_column: u32::try_from(attributes.get("column_end")?.as_u64()?).ok()?,
            })
        })?;
    let path = Path::new(&site.file);
    if path.is_absolute()
        && let Ok(relative) = path.strip_prefix(root)
    {
        site.file = relative.to_string_lossy().replace('\\', "/");
    }
    Some(site)
}

fn has_exact_remap_site(attributes: &serde_json::Map<String, serde_json::Value>) -> bool {
    attributes
        .get("source_anchor")
        .and_then(|value| serde_json::from_value::<SourceAnchor>(value.clone()).ok())
        .is_some_and(|site| site.is_valid() && site.start_byte < site.end_byte)
}

fn append_graph_diagnostics(
    extraction: &mut Extraction,
    prior: Option<serde_json::Value>,
    diagnostics: Vec<GraphDiagnostic>,
) {
    let mut values = extraction
        .extensions
        .remove(GRAPH_DIAGNOSTICS_EXTENSION)
        .and_then(|value| value.as_array().cloned())
        .unwrap_or_default();
    values.extend(
        prior
            .and_then(|value| value.as_array().cloned())
            .unwrap_or_default(),
    );
    values.extend(
        diagnostics
            .into_iter()
            .filter_map(|diagnostic| serde_json::to_value(diagnostic).ok()),
    );
    let mut remap_drops = Vec::new();
    let mut unrelated = Vec::new();
    let mut omitted = 0_usize;
    for value in values {
        match value
            .get("code")
            .and_then(serde_json::Value::as_str)
            .unwrap_or_default()
        {
            INCREMENTAL_REMAP_DROP_DIAGNOSTIC => remap_drops.push(value),
            INCREMENTAL_REMAP_TRUNCATION_DIAGNOSTIC => {
                omitted = omitted.saturating_add(remap_truncation_count(&value));
            }
            _ => unrelated.push(value),
        }
    }
    remap_drops.sort_by_cached_key(serde_json::Value::to_string);
    remap_drops.dedup();
    if remap_drops.len() > MAX_INCREMENTAL_REMAP_DIAGNOSTICS {
        omitted = omitted.saturating_add(
            remap_drops
                .len()
                .saturating_sub(MAX_INCREMENTAL_REMAP_DIAGNOSTICS),
        );
        remap_drops.truncate(MAX_INCREMENTAL_REMAP_DIAGNOSTICS);
    }
    unrelated.append(&mut remap_drops);
    if omitted > 0
        && let Ok(summary) = serde_json::to_value(GraphDiagnostic {
            severity: DiagnosticSeverity::Warning,
            code: INCREMENTAL_REMAP_TRUNCATION_DIAGNOSTIC.to_owned(),
            message: format!("omitted {omitted} additional incremental remap diagnostics"),
            anchor: None,
            related_ids: Vec::new(),
        })
    {
        unrelated.push(summary);
    }
    unrelated.sort_by_cached_key(serde_json::Value::to_string);
    unrelated.dedup();
    extraction.extensions.insert(
        GRAPH_DIAGNOSTICS_EXTENSION.to_owned(),
        serde_json::Value::Array(unrelated),
    );
}

fn remap_truncation_count(value: &serde_json::Value) -> usize {
    value
        .get("message")
        .and_then(serde_json::Value::as_str)
        .and_then(|message| {
            message
                .strip_prefix("omitted ")
                .and_then(|message| {
                    message.strip_suffix(" additional incremental remap diagnostics")
                })
                .and_then(|count| count.parse().ok())
        })
        .unwrap_or_default()
}

fn raw_node_match_key(node: &RawNodeRecord) -> Option<(String, String, String)> {
    Some((
        node.attributes.get("source_file")?.as_str()?.to_owned(),
        node.label().to_owned(),
        node.attributes
            .get("symbol_kind")
            .or_else(|| node.attributes.get("type"))
            .and_then(serde_json::Value::as_str)
            .unwrap_or("symbol")
            .to_owned(),
    ))
}

fn typed_node_match_key(
    node: &compass_model::code_graph::NodeRecord,
) -> Option<(String, String, String)> {
    Some((
        node.source_file()?.to_owned(),
        node.label().to_owned(),
        node.kind.as_str().to_owned(),
    ))
}

fn node_has_origin(node: &compass_model::code_graph::NodeRecord, origin: EvidenceOrigin) -> bool {
    node.evidence
        .iter()
        .any(|evidence| evidence.origin == origin)
}

fn is_semantic_layer_evidence(evidence: &Provenance) -> bool {
    evidence.extractor == SEMANTIC_LAYER_EXTRACTOR
}

fn node_has_semantic_layer_evidence(node: &compass_model::code_graph::NodeRecord) -> bool {
    node.evidence.iter().any(is_semantic_layer_evidence)
}

fn edge_has_semantic_layer_evidence(edge: &compass_model::code_graph::EdgeRecord) -> bool {
    edge.evidence.iter().any(is_semantic_layer_evidence)
}

fn canonical_source_set(paths: &[PathBuf], root: &Path) -> HashSet<PathBuf> {
    paths
        .iter()
        .map(|path| {
            let absolute = if path.is_absolute() {
                path.clone()
            } else {
                root.join(path)
            };
            canonical_identity(&absolute)
        })
        .collect()
}

fn source_in_set(
    value: Option<&serde_json::Value>,
    root: &Path,
    sources: &HashSet<PathBuf>,
) -> bool {
    let Some(source) = value.and_then(serde_json::Value::as_str) else {
        return false;
    };
    let path = Path::new(source);
    let absolute = if path.is_absolute() {
        path.to_path_buf()
    } else {
        root.join(path)
    };
    sources.contains(&canonical_identity(&absolute))
}

fn semantic_source_set(fragment: &serde_json::Value, root: &Path) -> HashSet<PathBuf> {
    ["nodes", "edges", "hyperedges"]
        .into_iter()
        .filter_map(|bucket| fragment.get(bucket).and_then(serde_json::Value::as_array))
        .flatten()
        .filter_map(|item| item.get("source_file").and_then(serde_json::Value::as_str))
        .map(|source| {
            let path = Path::new(source);
            let absolute = if path.is_absolute() {
                path.to_path_buf()
            } else {
                root.join(path)
            };
            canonical_identity(&absolute)
        })
        .collect()
}

fn stale_semantic_sources(
    graph_path: &Path,
    root: &Path,
    detected: &BTreeMap<String, Vec<String>>,
) -> HashSet<PathBuf> {
    let Ok(existing) = V1GraphDocument::load(graph_path) else {
        return HashSet::new();
    };
    let live = detected
        .values()
        .flatten()
        .map(|path| canonical_identity(Path::new(path)))
        .collect::<HashSet<_>>();
    let mut stale = existing
        .nodes
        .iter()
        .filter(|node| node_has_semantic_layer_evidence(node))
        .filter_map(|node| semantic_path_under_root(node.source_file(), root))
        .filter(|source| !live.contains(source))
        .collect::<HashSet<_>>();
    stale.extend(
        existing
            .links
            .iter()
            .filter(|edge| edge_has_semantic_layer_evidence(edge))
            .filter_map(|edge| semantic_path_under_root(edge.source_file(), root))
            .filter(|source| !live.contains(source)),
    );
    stale
}

fn semantic_path_under_root(source: Option<&str>, root: &Path) -> Option<PathBuf> {
    let source = source?;
    let path = Path::new(source);
    let absolute = if path.is_absolute() {
        path.to_path_buf()
    } else {
        root.join(path)
    };
    let identity = canonical_identity(&absolute);
    identity.starts_with(root).then_some(identity)
}

fn semantic_tokens(semantic: Option<&SemanticLayer>) -> (u64, u64) {
    let numeric = |key| {
        semantic
            .and_then(|layer| layer.fragment.get(key))
            .and_then(|value| {
                value
                    .as_u64()
                    .or_else(|| value.as_i64().and_then(|number| u64::try_from(number).ok()))
                    .or_else(|| value.as_f64().map(|number| number.max(0.0) as u64))
            })
            .unwrap_or_default()
    };
    (numeric("input_tokens"), numeric("output_tokens"))
}

pub(super) fn semantic_is_incomplete(layer: &SemanticLayer, root: &Path) -> bool {
    if !layer.partial_files.is_empty()
        || layer
            .fragment
            .get("failed_chunks")
            .and_then(serde_json::Value::as_u64)
            .is_some_and(|count| count > 0)
    {
        return true;
    }
    let extracted = semantic_source_set(&layer.fragment, root);
    canonical_source_set(&layer.refreshed_files, root)
        .iter()
        .any(|source| !extracted.contains(source))
}

fn save_build_manifest(
    manifest: &mut Manifest,
    files: &BTreeMap<String, Vec<String>>,
    path: &Path,
    root: &Path,
    semantic: Option<&SemanticLayer>,
) -> Result<(), CoreError> {
    let Some(layer) = semantic.filter(|layer| !semantic_layer_is_empty(layer)) else {
        let scan_corpus = files.values().flatten().cloned().collect::<BTreeSet<_>>();
        manifest.save(
            files,
            path,
            ManifestKind::Ast,
            Some(root),
            Some(&scan_corpus),
            None,
        )?;
        return Ok(());
    };

    let extracted = semantic_source_set(&layer.fragment, root);
    let partial = canonical_source_set(&layer.partial_files, root);
    let semantic_types = ["document", "paper", "image"];
    let stamped = files
        .iter()
        .map(|(file_type, bucket)| {
            let retained = bucket
                .iter()
                .filter(|file| {
                    if !semantic_types.contains(&file_type.as_str()) {
                        return true;
                    }
                    let canonical = canonical_identity(Path::new(file));
                    extracted.contains(&canonical) && !partial.contains(&canonical)
                })
                .cloned()
                .collect();
            (file_type.clone(), retained)
        })
        .collect::<BTreeMap<_, _>>();
    let scan_corpus = files.values().flatten().cloned().collect::<BTreeSet<_>>();
    let successfully_stamped = stamped
        .values()
        .flatten()
        .map(|file| canonical_identity(Path::new(file)))
        .collect::<HashSet<_>>();
    let clear_semantic = layer
        .refreshed_files
        .iter()
        .map(|file| {
            let absolute = if file.is_absolute() {
                file.clone()
            } else {
                root.join(file)
            };
            canonical_identity(&absolute)
        })
        .filter(|file| !successfully_stamped.contains(file))
        .map(|file| file.to_string_lossy().into_owned())
        .collect::<BTreeSet<_>>();
    manifest.save(
        &stamped,
        path,
        ManifestKind::Both,
        Some(root),
        Some(&scan_corpus),
        Some(&clear_semantic),
    )?;
    Ok(())
}

fn semantic_document_sources(graph_path: &Path, root: &Path) -> HashSet<PathBuf> {
    let Ok(existing) = V1GraphDocument::load(graph_path) else {
        return HashSet::new();
    };
    existing
        .nodes
        .into_iter()
        .filter(|node| node_has_semantic_layer_evidence(node) && node.kind == NodeKind::Resource)
        .filter_map(|node| {
            node.source_file().map(Path::new).map(|path| {
                if path.is_absolute() {
                    canonical_identity(path)
                } else {
                    canonical_identity(&root.join(path))
                }
            })
        })
        .collect()
}

fn canonical_identity(path: &Path) -> PathBuf {
    fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf())
}

fn graph_configuration_digest(
    options: &BuildOptions,
    output_dir: &Path,
) -> Result<String, CoreError> {
    let bytes = serde_json::to_vec(&build_profile(options)).map_err(|source| {
        CoreError::SerializeExtraction {
            path: output_dir.join("graph.json"),
            source,
        }
    })?;
    Ok(format!("sha256:{:x}", Sha256::digest(bytes)))
}

fn detection_inventory(
    detection: &Detection,
    semantic: Option<&SemanticLayer>,
    extraction_failures: &BTreeMap<PathBuf, String>,
    extraction_partials: &BTreeMap<PathBuf, String>,
    root: &Path,
) -> Vec<InventoryEvidence> {
    let mut inventory = detection
        .files
        .iter()
        .flat_map(|(category, paths)| {
            paths.iter().map(move |path| {
                let absolute = if Path::new(path).is_absolute() {
                    PathBuf::from(path)
                } else {
                    root.join(path)
                };
                let identity = canonical_identity(&absolute);
                let spec = Registry::resolve(Path::new(path));
                let status = if extraction_failures.contains_key(&identity) {
                    ExtractionStatus::ParseFailure
                } else if extraction_partials.contains_key(&identity)
                    || semantic.is_some_and(|layer| {
                        layer.partial_files.iter().any(|partial| {
                            canonical_identity(partial) == canonical_identity(&absolute)
                        })
                    })
                {
                    ExtractionStatus::Partial
                } else if source_is_generated(&absolute) {
                    ExtractionStatus::Generated
                } else {
                    match category.as_str() {
                        "image" | "video" => ExtractionStatus::Binary,
                        "code" | "document" if spec.is_some() => ExtractionStatus::Extracted,
                        _ => ExtractionStatus::Unsupported,
                    }
                };
                InventoryEvidence {
                    path: PathBuf::from(path),
                    language: spec.map(|spec| spec.name.to_owned()),
                    producer: spec.map_or_else(
                        || "compass.files.detect".to_owned(),
                        |spec| format!("compass.languages.{}", spec.name),
                    ),
                    status,
                    reason: extraction_failures
                        .get(&identity)
                        .or_else(|| extraction_partials.get(&identity))
                        .cloned()
                        .or_else(|| {
                            (status == ExtractionStatus::Unsupported)
                                .then(|| format!("no extractor for detected {category} file"))
                        }),
                }
            })
        })
        .collect::<Vec<_>>();
    inventory.extend(detection.unclassified.iter().map(|path| InventoryEvidence {
        path: PathBuf::from(path),
        language: None,
        producer: "compass.files.detect".to_owned(),
        status: ExtractionStatus::Unsupported,
        reason: Some("unclassified by detector".to_owned()),
    }));
    inventory.extend(detection.ignored.iter().map(|path| InventoryEvidence {
        path: PathBuf::from(path),
        language: None,
        producer: "compass.files.detect".to_owned(),
        status: ExtractionStatus::Excluded,
        reason: Some("excluded by ignore policy".to_owned()),
    }));
    inventory
}

fn prepare_extraction_for_publication(
    path: &Path,
    extraction: &mut Extraction,
    root: &Path,
    failures: &mut BTreeMap<PathBuf, String>,
    partials: &mut BTreeMap<PathBuf, String>,
) {
    let identity = canonical_identity(path);
    make_framework_fact_sources_portable(extraction, root);
    if let Some(error) = extraction.error.take() {
        failures.insert(identity, portable_diagnostic_reason(&error, path, root));
        *extraction = Extraction::default();
        extraction.raw_calls = None;
        return;
    }
    if extraction
        .extensions
        .get(EXTRACTION_QUALITY_EXTENSION)
        .and_then(serde_json::Value::as_str)
        != Some(EXTRACTION_QUALITY_PARTIAL)
    {
        return;
    }
    let reason = extraction
        .extensions
        .get(EXTRACTION_QUALITY_REASON_EXTENSION)
        .and_then(serde_json::Value::as_str)
        .unwrap_or("extraction completed with parser recovery");
    partials.insert(identity, portable_diagnostic_reason(reason, path, root));
    extraction.edges.clear();
    extraction.hyperedges.clear();
    extraction.framework_facts.clear();
    extraction.raw_calls = None;
}

fn make_framework_fact_sources_portable(extraction: &mut Extraction, root: &Path) {
    if extraction.semantic_evidence.is_none() {
        return;
    }
    for fact in &mut extraction.framework_facts {
        let source_file = match fact {
            RawFrameworkFact::Route(route) => &mut route.anchor.source_file,
            RawFrameworkFact::Domain(domain) => &mut domain.anchor.source_file,
            RawFrameworkFact::Annotation(annotation) => &mut annotation.anchor.source_file,
        };
        let path = Path::new(source_file);
        if !path.is_absolute() {
            *source_file = source_file.replace('\\', "/");
            continue;
        }
        let canonical = canonicalize_allow_missing(path);
        *source_file = canonical.strip_prefix(root).map_or_else(
            |_| portable_out_of_root_source(&canonical, root),
            |relative| relative.to_string_lossy().replace('\\', "/"),
        );
    }
}

fn portable_diagnostic_reason(reason: &str, path: &Path, root: &Path) -> String {
    let root_text = root.to_string_lossy();
    let path_text = path.to_string_lossy();
    let relative = path
        .strip_prefix(root)
        .unwrap_or(path)
        .to_string_lossy()
        .replace('\\', "/");
    let sanitized = reason
        .replace(root_text.as_ref(), ".")
        .replace(path_text.as_ref(), &relative)
        .replace('\\', "/");
    sanitized.chars().take(512).collect()
}

fn source_is_generated(path: &Path) -> bool {
    fs::read(path).ok().is_some_and(|bytes| {
        let head = String::from_utf8_lossy(&bytes[..bytes.len().min(2_048)]);
        ["DO NOT EDIT", "@generated", "Generated by"]
            .iter()
            .any(|marker| head.contains(marker))
    })
}

fn source_was_deleted(value: Option<&serde_json::Value>, root: &Path) -> bool {
    let Some(source) = value.and_then(serde_json::Value::as_str) else {
        return false;
    };
    let path = Path::new(source);
    let absolute = if path.is_absolute() {
        path.to_path_buf()
    } else {
        root.join(path)
    };
    Registry::resolve(path).is_some() && !absolute.exists()
}

fn report_root_label(path: &Path) -> String {
    if path.is_absolute() {
        return path
            .file_name()
            .map(|value| value.to_string_lossy().into_owned())
            .filter(|value| !value.is_empty())
            .unwrap_or_else(|| path.to_string_lossy().into_owned());
    }
    if path == Path::new(".") {
        return std::env::current_dir()
            .ok()
            .and_then(|directory| directory.file_name().map(|value| value.to_owned()))
            .map(|value| value.to_string_lossy().into_owned())
            .unwrap_or_else(|| ".".to_owned());
    }
    path.to_string_lossy().into_owned()
}

fn topology_is_unchanged(existing: &GraphDocument, candidate: &GraphDocument) -> bool {
    if existing.directed != candidate.directed || existing.multigraph != candidate.multigraph {
        return false;
    }
    let mut existing_nodes = existing
        .nodes
        .iter()
        .map(canonical_node)
        .collect::<Vec<_>>();
    let mut candidate_nodes = candidate
        .nodes
        .iter()
        .map(canonical_node)
        .collect::<Vec<_>>();
    existing_nodes.sort();
    candidate_nodes.sort();
    if existing_nodes != candidate_nodes {
        return false;
    }
    let mut existing_edges = existing
        .links
        .iter()
        .map(canonical_edge)
        .collect::<Vec<_>>();
    let mut candidate_edges = candidate
        .links
        .iter()
        .map(canonical_edge)
        .collect::<Vec<_>>();
    existing_edges.sort();
    candidate_edges.sort();
    existing_edges == candidate_edges
        && canonical_hyperedges(existing) == canonical_hyperedges(candidate)
}

fn update_artifacts_complete(options: &BuildOptions, output_dir: &Path) -> bool {
    let mut required = vec![
        "graph.json",
        "GRAPH_REPORT.md",
        "orientation.json",
        "labels.json",
        "source-root.txt",
        GRAPH_OVERVIEW_FILE,
    ];
    if options.graph_storage.publishes_store() {
        required.push(STORE_REF_FILE_NAME);
    }
    required
        .into_iter()
        .all(|name| output_dir.join(name).is_file())
}

fn storage_artifacts_complete(graph_storage: GraphStorage, output_dir: &Path) -> bool {
    !graph_storage.publishes_store() || store_artifact_complete(output_dir)
}

fn store_artifact_complete(output_dir: &Path) -> bool {
    let graph_path = output_dir.join("graph.json");
    let path = local_sqlite_store_path(&graph_path);
    let reference_path = output_dir.join(STORE_REF_FILE_NAME);
    let Ok(reference_bytes) = fs::read(reference_path) else {
        return false;
    };
    let Ok(reference) = serde_json::from_slice::<StoreRef>(&reference_bytes) else {
        return false;
    };
    reference.validate().is_ok()
        && path.is_file()
        && SqliteStore::open_read_only(path).is_ok_and(|store| {
            store
                .graph_snapshot_reference_for(&reference.snapshot_id, &reference.manifest_digest)
                .is_ok_and(|actual| actual == reference)
        })
}

pub(crate) fn write_graph_overview_artifact(
    document: &GraphDocument,
    communities: &compass_graph::Communities,
    labels: &BTreeMap<usize, String>,
    output_dir: &Path,
) -> Result<(), CoreError> {
    let overview = graph_overview_model(document, communities, labels, output_dir)?;
    write_prepared_graph_overview(overview, output_dir)
}

fn graph_overview_model(
    document: &GraphDocument,
    communities: &compass_graph::Communities,
    labels: &BTreeMap<usize, String>,
    output_dir: &Path,
) -> Result<Option<GraphViewModel>, CoreError> {
    Ok(graph_view_model_document(
        document,
        communities,
        output_dir.join("graph.json"),
        &HtmlOptions {
            community_labels: (!labels.is_empty()).then_some(labels),
            node_limit: Some(GRAPH_OVERVIEW_NODE_LIMIT),
            ..HtmlOptions::default()
        },
    )?)
}

fn write_prepared_graph_overview(
    overview: Option<GraphViewModel>,
    output_dir: &Path,
) -> Result<(), CoreError> {
    let overview_path = output_dir.join(GRAPH_OVERVIEW_FILE);
    if let Some(model) = overview {
        let graph_path = output_dir.join("graph.json");
        let source_graph_bytes = fs::metadata(&graph_path)
            .map_err(|source| compass_files::FileError::Io {
                path: graph_path,
                source,
            })?
            .len();
        write_json_atomic(
            &overview_path,
            &json!({
                "schema": GRAPH_OVERVIEW_SCHEMA,
                "sourceGraphBytes": source_graph_bytes,
                "nodeLimit": GRAPH_OVERVIEW_NODE_LIMIT,
                "model": model,
            }),
            false,
        )?;
    } else {
        remove_if_exists(&overview_path)?;
    }
    Ok(())
}

fn unchanged_output_stats(options: &BuildOptions, output_dir: &Path) -> Option<OutputStats> {
    let graph_path = output_dir.join("graph.json");
    let graph_bytes = fs::metadata(&graph_path).ok()?.len();
    let saved = fs::read(output_dir.join(OUTPUT_STATS_FILE))
        .ok()
        .and_then(|bytes| serde_json::from_slice::<OutputStats>(&bytes).ok());
    if let Some(stats) = saved.filter(|stats| stats.graph_bytes == graph_bytes) {
        if options.no_cluster == stats.clustered
            || !output_dir.join("source-root.txt").is_file()
            || (!options.no_cluster
                && (options.resolution != 1.0
                    || options.exclude_hubs.is_some()
                    || !update_artifacts_complete(options, output_dir)))
            || (!options.no_viz && !output_dir.join("graph.html").is_file() && stats.nodes <= 5_000)
        {
            return None;
        }
        return Some(stats);
    }

    let bytes = fs::read(&graph_path).ok()?;
    let header = &bytes[..bytes.len().min(512)];
    let has_key = |key: &[u8]| header.windows(key.len()).any(|window| window == key);
    let is_clustered = has_key(b"\"directed\"") && has_key(b"\"multigraph\"");
    if options.no_cluster == is_clustered || !output_dir.join("source-root.txt").is_file() {
        return None;
    }
    let document: GraphDocument = serde_json::from_slice(&bytes).ok()?;
    if options.no_cluster {
        let stats = OutputStats {
            graph_bytes,
            nodes: document.nodes.len(),
            edges: document.links.len(),
            communities: 0,
            clustered: false,
            omitted_nodes: 0,
            omitted_edges: 0,
            identity_collisions: 0,
        };
        let _ = write_json_atomic(output_dir.join(OUTPUT_STATS_FILE), &stats, true);
        return Some(stats);
    }
    if options.resolution != 1.0
        || options.exclude_hubs.is_some()
        || !update_artifacts_complete(options, output_dir)
    {
        return None;
    }
    if !options.no_viz && !output_dir.join("graph.html").is_file() && document.nodes.len() <= 5_000
    {
        return None;
    }
    let stats = OutputStats {
        graph_bytes,
        nodes: document.nodes.len(),
        edges: document.links.len(),
        communities: document
            .nodes
            .iter()
            .filter_map(|node| node.attributes.get("community")?.as_u64())
            .collect::<HashSet<_>>()
            .len(),
        clustered: true,
        omitted_nodes: 0,
        omitted_edges: 0,
        identity_collisions: 0,
    };
    let _ = write_json_atomic(output_dir.join(OUTPUT_STATS_FILE), &stats, true);
    Some(stats)
}

fn saved_publication_omissions(output_dir: &Path) -> PublicationOmissions {
    fs::read(output_dir.join(OUTPUT_STATS_FILE))
        .ok()
        .and_then(|bytes| serde_json::from_slice::<OutputStats>(&bytes).ok())
        .map_or_else(PublicationOmissions::default, |stats| stats.omissions())
}

fn save_output_stats(
    output_dir: &Path,
    nodes: usize,
    edges: usize,
    communities: usize,
    clustered: bool,
    omissions: PublicationOmissions,
) -> Result<(), CoreError> {
    let graph_bytes = fs::metadata(output_dir.join("graph.json"))
        .map_err(|source| compass_files::FileError::Io {
            path: output_dir.join("graph.json"),
            source,
        })?
        .len();
    write_json_atomic(
        output_dir.join(OUTPUT_STATS_FILE),
        &OutputStats {
            graph_bytes,
            nodes,
            edges,
            communities,
            clustered,
            omitted_nodes: omissions.nodes,
            omitted_edges: omissions.edges,
            identity_collisions: omissions.identity_collisions,
        },
        true,
    )?;
    Ok(())
}

fn canonical_node(node: &NodeRecord) -> String {
    let mut value = node.attributes.clone();
    for key in ["community", "community_name", "norm_label"] {
        value.remove(key);
    }
    value.insert("id".to_owned(), serde_json::Value::String(node.id.clone()));
    serde_json::to_string(&value).unwrap_or_default()
}

fn canonical_edge(edge: &EdgeRecord) -> String {
    let mut value = edge.attributes.clone();
    let source = value
        .remove("_src")
        .and_then(|value| value.as_str().map(str::to_owned))
        .unwrap_or_else(|| edge.source.clone());
    let target = value
        .remove("_tgt")
        .and_then(|value| value.as_str().map(str::to_owned))
        .unwrap_or_else(|| edge.target.clone());
    value.remove("confidence_score");
    value.insert("source".to_owned(), serde_json::Value::String(source));
    value.insert("target".to_owned(), serde_json::Value::String(target));
    serde_json::to_string(&value).unwrap_or_default()
}

fn canonical_hyperedges(document: &GraphDocument) -> Vec<String> {
    let mut values = document
        .extras
        .get("hyperedges")
        .or_else(|| document.graph.get("hyperedges"))
        .and_then(serde_json::Value::as_array)
        .into_iter()
        .flatten()
        .map(|value| serde_json::to_string(value).unwrap_or_default())
        .collect::<Vec<_>>();
    values.sort();
    values
}

#[derive(Debug, Deserialize)]
struct CommunityScanDocument {
    #[serde(default)]
    nodes: Vec<CommunityScanNode>,
}

#[derive(Debug, Deserialize)]
struct CommunityScanNode {
    id: String,
    #[serde(default)]
    community: Option<Value>,
}

fn previous_communities(path: &Path) -> HashMap<String, usize> {
    if V1GraphDocument::size_cap_exceeded(path).is_some() {
        return HashMap::new();
    }
    let Ok(file) = fs::File::open(path) else {
        return HashMap::new();
    };
    let Ok(document) = serde_json::from_reader::<_, CommunityScanDocument>(BufReader::new(file))
    else {
        return HashMap::new();
    };
    document
        .nodes
        .into_iter()
        .filter_map(|node| {
            let value = node.community?;
            let community = value
                .as_u64()
                .or_else(|| value.get("id").and_then(Value::as_u64))?;
            Some((node.id, usize::try_from(community).ok()?))
        })
        .collect()
}

pub(crate) fn remove_if_exists(path: &Path) -> Result<(), CoreError> {
    match fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(source) => Err(compass_files::FileError::Io {
            path: path.to_path_buf(),
            source,
        }
        .into()),
    }
}

fn absolutize(path: &Path) -> PathBuf {
    if path.is_absolute() {
        path.to_path_buf()
    } else {
        std::env::current_dir().map_or_else(|_| path.to_path_buf(), |cwd| cwd.join(path))
    }
}

fn profile_internal(label: &str, started: &mut Instant) {
    if std::env::var_os("COMPASS_PROFILE_INTERNAL").is_some() {
        eprintln!(
            "[compass internal] {label}: {:.3}s",
            started.elapsed().as_secs_f64()
        );
    }
    *started = Instant::now();
}

fn profile_internal_duration(label: &str, elapsed: Duration) {
    if std::env::var_os("COMPASS_PROFILE_INTERNAL").is_some() {
        eprintln!("[compass internal] {label}: {:.3}s", elapsed.as_secs_f64());
    }
}

fn profile_extraction_inventory(extractions: &[Extraction]) {
    if std::env::var_os("COMPASS_PROFILE_INTERNAL").is_none() {
        return;
    }
    let mut declarations = 0_usize;
    let mut scopes = 0_usize;
    let mut bindings = 0_usize;
    let mut occurrences = 0_usize;
    let mut candidates = 0_usize;
    let mut external_candidates = 0_usize;
    let mut hierarchy_candidates = 0_usize;
    let mut relations = BTreeMap::<&'static str, usize>::new();
    for batch in extractions
        .iter()
        .filter_map(|extraction| extraction.semantic_evidence.as_ref())
    {
        declarations = declarations.saturating_add(batch.declarations.len());
        scopes = scopes.saturating_add(batch.scopes.len());
        bindings = bindings.saturating_add(batch.bindings.len());
        occurrences = occurrences.saturating_add(batch.occurrences.len());
        candidates = candidates.saturating_add(batch.candidates.len());
        for candidate in &batch.candidates {
            if candidate.constraints.allow_external {
                external_candidates = external_candidates.saturating_add(1);
            }
            if candidate.constraints.hierarchy.is_some() {
                hierarchy_candidates = hierarchy_candidates.saturating_add(1);
            }
            let relation = match candidate.relation {
                compass_languages::CandidateRelation::Calls => "calls",
                compass_languages::CandidateRelation::IndirectCalls => "indirect_calls",
                compass_languages::CandidateRelation::Tests => "tests",
                compass_languages::CandidateRelation::References => "references",
                compass_languages::CandidateRelation::Contains => "contains",
                compass_languages::CandidateRelation::Owns => "owns",
                _ => "other",
            };
            *relations.entry(relation).or_default() += 1;
        }
    }
    eprintln!(
        "[compass internal] extraction inventory: raw_nodes={} raw_edges={} declarations={declarations} scopes={scopes} bindings={bindings} occurrences={occurrences} candidates={candidates} external_candidates={external_candidates} hierarchy_candidates={hierarchy_candidates} relations={relations:?}",
        extractions
            .iter()
            .map(|extraction| extraction.nodes.len())
            .sum::<usize>(),
        extractions
            .iter()
            .map(|extraction| extraction.edges.len())
            .sum::<usize>(),
    );
}

fn join_program_worker(
    handle: Option<std::thread::JoinHandle<Result<(ProgramBuildSummary, Duration), CoreError>>>,
    timings: &mut BuildTimings,
) -> Result<Option<ProgramBuildSummary>, CoreError> {
    let Some(handle) = handle else {
        return Ok(None);
    };
    let (program, elapsed) = handle
        .join()
        .map_err(|_| CoreError::WorkerPanic("program analysis".to_owned()))??;
    timings.program_analysis = elapsed;
    Ok(Some(program))
}

pub(crate) fn git_commit(root: &Path) -> Option<String> {
    let dot_git = root
        .ancestors()
        .map(|directory| directory.join(".git"))
        .find(|candidate| candidate.exists())?;
    let repository = dot_git.parent()?.to_path_buf();
    let git_dir = if dot_git.is_dir() {
        dot_git
    } else {
        let text = fs::read_to_string(&dot_git).ok()?;
        let relative = text.trim().strip_prefix("gitdir:")?.trim();
        absolutize_from(&repository, Path::new(relative))
    };
    let head = fs::read_to_string(git_dir.join("HEAD")).ok()?;
    let head = head.trim();
    if let Some(reference) = head.strip_prefix("ref: ") {
        fs::read_to_string(git_dir.join(reference))
            .ok()
            .map(|value| value.trim().to_owned())
    } else if !head.is_empty() {
        Some(head.to_owned())
    } else {
        None
    }
}

fn absolutize_from(root: &Path, path: &Path) -> PathBuf {
    if path.is_absolute() {
        path.to_path_buf()
    } else {
        root.join(path)
    }
}

fn read_source_text_with_limit(path: &Path, max_source_bytes: u64) -> Option<(String, String)> {
    fs::metadata(path)
        .ok()
        .filter(|metadata| metadata.is_file() && metadata.len() <= max_source_bytes)
        .and_then(|_| fs::read(path).ok())
        .map(|bytes| {
            (
                path.to_string_lossy().into_owned(),
                String::from_utf8_lossy(&bytes).into_owned(),
            )
        })
}

fn cached_framework_evidence_matches(
    extraction: &Extraction,
    path: &Path,
    project_evidence: &ProjectEvidenceIndex,
) -> bool {
    extraction
        .extensions
        .get(FRAMEWORK_PROJECT_EVIDENCE_EXTENSION)
        .and_then(serde_json::Value::as_str)
        == Some(project_evidence.fingerprint_for(path))
}

fn cached_universal_evidence_matches(extraction: &Extraction, path: &Path) -> bool {
    let Some(profile) = Registry::universal_adapter(path) else {
        return true;
    };
    extraction.raw_calls.is_none()
        && extraction.semantic_evidence.as_ref().is_some_and(|batch| {
            batch.adapter.id == profile.id
                && batch.adapter.language == profile.language
                && batch.adapter.version == profile.version
                && batch.adapter.evidence_schema == profile.evidence_schema
                && batch.adapter.profile == profile.profile
                && batch.adapter.capabilities.as_slice() == profile.capabilities
                && compass_languages::validate_evidence(
                    batch,
                    compass_languages::EvidenceLimits::default(),
                )
                .is_ok()
        })
}

fn extraction_has_cacheable_ast_facts(extraction: &Extraction) -> bool {
    !extraction.nodes.is_empty()
        || !extraction.edges.is_empty()
        || !extraction.hyperedges.is_empty()
        || extraction
            .raw_calls
            .as_ref()
            .is_some_and(|calls| !calls.is_empty())
        || !extraction.framework_facts.is_empty()
        || extraction.semantic_evidence.as_ref().is_some_and(|batch| {
            !batch.declarations.is_empty()
                || !batch.scopes.is_empty()
                || !batch.bindings.is_empty()
                || !batch.occurrences.is_empty()
                || !batch.candidates.is_empty()
                || !batch.diagnostics.is_empty()
        })
}

#[cfg(test)]
mod tests {
    use std::error::Error;

    use compass_graph::{GraphSnapshotReader, IndexKind};
    use compass_model::code_graph::GraphDocument as V1GraphDocument;
    use compass_model::provenance::{EvidenceConfidence, effective_confidence};
    use serde_json::{Map, Value};

    use super::*;

    #[test]
    fn current_orientation_health_preserves_scope_and_partial_publication() {
        let mut options = BuildOptions::new(".");
        options.scope.include = vec!["src/".to_owned()];
        options.scope.exclude = vec!["src/generated/".to_owned()];
        options.extra_excludes = vec!["vendor".to_owned(), "src/generated/".to_owned()];
        options.code_only = true;
        let health = current_orientation_health(
            &options,
            PublicationOmissions {
                nodes: 7,
                edges: 11,
                identity_collisions: 2,
                examples_omitted: 3,
            },
        );
        assert_eq!(health.freshness, FreshnessStatus::Current);
        assert_eq!(
            health.freshness_basis,
            FreshnessBasis::JustBuiltSelectedInputs
        );
        assert_eq!(health.publication, Some(PublicationStatus::Partial));
        assert_eq!(health.omitted_nodes, Some(7));
        assert_eq!(health.omitted_edges, Some(11));
        assert_eq!(health.identity_collisions, Some(2));
        assert_eq!(health.diagnostic_examples_omitted, Some(3));
        assert_eq!(health.scope_includes, ["src/"]);
        assert_eq!(health.configured_exclusions, ["src/generated/", "vendor"]);
        assert!(health.corpus_measurements_available);
        assert!(
            health.build_profile.as_deref().is_some_and(|value| {
                value.contains("update") && value.contains("code_only=true")
            })
        );
    }

    #[test]
    fn inference_level_changes_profile_identity_without_changing_the_default_profile() {
        let default = BuildOptions::new(".");
        let default_profile = build_profile(&default);
        let default_json = serde_json::to_value(&default_profile).unwrap_or_default();
        assert!(default_json.get("inference_level").is_none());

        let mut medium = default;
        medium.inference_level = InferenceLevel::Medium;
        let medium_profile = build_profile(&medium);
        assert_eq!(
            serde_json::to_value(&medium_profile)
                .unwrap_or_default()
                .get("inference_level"),
            Some(&Value::String("medium".to_owned()))
        );
        assert_ne!(default_profile, medium_profile);
        assert!(
            current_orientation_health(&medium, PublicationOmissions::default())
                .build_profile
                .as_deref()
                .is_some_and(|profile| profile.contains("inference=medium"))
        );
    }

    #[test]
    fn build_inference_levels_publish_nested_coherent_graphs() -> Result<(), Box<dyn Error>> {
        let directory = tempfile::tempdir()?;
        let root = directory.path().join("source");
        fs::create_dir(&root)?;
        fs::write(
            root.join("lib.rs"),
            r#"
                use external_crate::ExternalType;

                struct LocalService;

                impl LocalService {
                    fn call() {}
                }

                fn run(value: ExternalType) {
                    value.execute();
                    external_crate::Service::call();
                }

                #[test]
                fn test_run() {
                    LocalService::call();
                    external_crate::Service::call();
                }
            "#,
        )?;

        let mut previous = None;
        let mut low_graph = None;
        let mut max_graph = None;
        for level in [
            InferenceLevel::Low,
            InferenceLevel::Medium,
            InferenceLevel::High,
            InferenceLevel::Max,
        ] {
            let mut options = BuildOptions::new(&root);
            options.output_root = Some(directory.path().join("artifacts"));
            options.graph_storage = GraphStorage::Json;
            options.inference_level = level;
            options.no_cluster = true;
            options.no_viz = true;
            let result = build_local_graph(&options)?;
            let document = V1GraphDocument::load(&result.output_dir.join("graph.json"))?;
            let inferred = document
                .links
                .iter()
                .filter(|edge| {
                    effective_confidence(&edge.evidence) == Some(EvidenceConfidence::Inferred)
                })
                .count();
            if level == InferenceLevel::Low {
                assert_eq!(inferred, 0);
                low_graph = Some((document.nodes.clone(), document.links.clone()));
            } else if level == InferenceLevel::Max {
                max_graph = Some(document.clone());
            }
            if let Some((nodes, edges, inferred_before)) = previous {
                assert!(document.nodes.len() >= nodes);
                assert!(document.links.len() >= edges);
                assert!(inferred >= inferred_before);
            }
            previous = Some((document.nodes.len(), document.links.len(), inferred));
        }
        let (_, _, max_inferred) = previous.ok_or("inference levels were not built")?;
        assert!(max_inferred > 0);
        let (low_nodes, low_edges) = low_graph.ok_or("low inference graph was not built")?;
        let mut filtered_max = max_graph.ok_or("max inference graph was not built")?;
        apply_inference_level(&mut filtered_max, InferenceLevel::Low);
        assert_eq!(low_nodes, filtered_max.nodes);
        assert_eq!(low_edges, filtered_max.links);
        Ok(())
    }

    #[test]
    fn current_report_preserves_detection_warning_for_every_build_purpose() {
        let warning = "small corpus warning".to_owned();
        let summary = report_detection_summary(4, 99, Some(warning.clone()));
        assert_eq!(summary.total_files, 4);
        assert_eq!(summary.total_words, 99);
        assert_eq!(summary.warning, Some(warning));
    }

    #[test]
    fn force_cache_reuse_never_authorizes_prior_published_graph_input() {
        assert!(cache_reuse_enabled(false, false));
        assert!(cache_reuse_enabled(false, true));
        assert!(!cache_reuse_enabled(true, false));
        assert!(cache_reuse_enabled(true, true));
        assert!(prior_published_graph_input_enabled(false));
        assert!(!prior_published_graph_input_enabled(true));
    }

    #[test]
    fn explicit_multiple_ast_workers_enable_small_project_parallelism() {
        let mut options = BuildOptions::new(".");
        assert!(!should_parallel_extract(&options, 31));
        assert!(should_parallel_extract(&options, 32));

        options.max_workers = Some(2);
        assert!(should_parallel_extract(&options, 1));

        options.max_workers = Some(1);
        assert!(!should_parallel_extract(&options, 31));
        assert!(should_parallel_extract(&options, 32));
    }

    #[test]
    fn default_ast_workers_stay_within_the_memory_bound() {
        assert_eq!(default_ast_worker_cap(0), DEFAULT_AST_WORKER_CAP);
        assert_eq!(
            default_ast_worker_cap(LARGE_REPOSITORY_AST_WORKER_MIN_FILES - 1),
            DEFAULT_AST_WORKER_CAP
        );
        assert_eq!(
            default_ast_worker_cap(LARGE_REPOSITORY_AST_WORKER_MIN_FILES),
            LARGE_REPOSITORY_AST_WORKER_CAP
        );
        assert!(default_ast_workers(0) > 0);
        assert!(default_ast_workers(0) <= DEFAULT_AST_WORKER_CAP);
        assert!(default_ast_workers(LARGE_REPOSITORY_AST_WORKER_MIN_FILES) > 0);
        assert!(
            default_ast_workers(LARGE_REPOSITORY_AST_WORKER_MIN_FILES)
                <= LARGE_REPOSITORY_AST_WORKER_CAP
        );
    }

    #[test]
    fn pipeline_rayon_workers_use_a_bounded_default_and_explicit_override() {
        let mut options = BuildOptions::new(".");
        let default_workers = pipeline_rayon_workers(&options);
        assert_eq!(default_workers, default_pipeline_rayon_workers());
        assert!(default_workers > 0);
        assert!(default_workers <= PIPELINE_RAYON_WORKER_CAP);

        options.max_workers = Some(2);
        assert_eq!(pipeline_rayon_workers(&options), 2);

        options.max_workers = Some(0);
        assert_eq!(pipeline_rayon_workers(&options), 1);
    }

    #[test]
    fn parallel_ast_fact_digests_match_source_ordered_contract() -> Result<(), Box<dyn Error>> {
        let root = Path::new("/repo");
        let paths = (0..300)
            .map(|index| root.join(format!("src/module_{index:03}.py")))
            .collect::<Vec<_>>();
        let extractions = (0..paths.len())
            .map(|index| {
                let mut extraction = Extraction::default();
                extraction
                    .extensions
                    .insert("ordinal".to_owned(), json!(index));
                extraction
            })
            .collect::<Vec<_>>();
        let expected = paths
            .iter()
            .zip(&extractions)
            .map(|(path, extraction)| {
                Ok((
                    relative_fact_path(path, root),
                    extraction_fact_digest(extraction)?,
                ))
            })
            .collect::<Result<BTreeMap<_, _>, serde_json::Error>>()?;

        let actual = ast_fact_digest_entries(&paths, &extractions, root, root)?;

        assert_eq!(actual, expected);
        assert_eq!(
            actual.keys().next().map(String::as_str),
            Some("src/module_000.py")
        );
        assert_eq!(
            actual.keys().next_back().map(String::as_str),
            Some("src/module_299.py")
        );
        Ok(())
    }

    #[test]
    fn ast_fact_digests_parallelize_at_the_cold_extraction_crossover() {
        assert!(!should_parallel_ast_fact_digest(
            PARALLEL_AST_FACT_DIGEST_MIN_FILES - 1
        ));
        assert!(should_parallel_ast_fact_digest(
            PARALLEL_AST_FACT_DIGEST_MIN_FILES
        ));
        assert!(should_parallel_ast_fact_digest(246));
    }

    #[cfg(unix)]
    #[test]
    fn detected_file_sets_match_canonicalizes_manifest_symlink_aliases()
    -> Result<(), Box<dyn Error>> {
        use std::os::unix::fs::symlink;

        let directory = tempfile::tempdir()?;
        let source_directory = directory.path().join("src");
        fs::create_dir_all(&source_directory)?;
        let source = source_directory.join("main.py");
        fs::write(&source, "def main():\n    return 1\n")?;
        let alias = source_directory.join("main-alias.py");
        symlink("main.py", &alias)?;
        let manifest_path = directory.path().join("manifest.json");
        fs::write(
            &manifest_path,
            serde_json::to_vec(&json!({
                "src/main.py": {"mtime": 1.0, "ast_hash": "hash"},
                "src/main-alias.py": {"mtime": 1.0, "ast_hash": "hash"}
            }))?,
        )?;
        let manifest = Manifest::load(&manifest_path, Some(directory.path()));
        let files = BTreeMap::from([(
            "code".to_owned(),
            vec![source.to_string_lossy().into_owned()],
        )]);

        assert!(detected_file_sets_match(&manifest, &files));
        Ok(())
    }

    #[test]
    fn resolver_source_text_is_scoped_to_php_files() {
        assert!(is_php_source_path(Path::new("src/controller.PHP")));
        assert!(!is_php_source_path(Path::new("src/controller.py")));
        assert!(!is_php_source_path(Path::new("src/controller")));
    }

    #[test]
    fn compact_extraction_preserves_nested_ast_facts() -> Result<(), Box<dyn Error>> {
        let mut attributes = Map::with_capacity(64);
        for index in 0..32 {
            attributes.insert(format!("attribute_{index}"), json!(index));
        }
        attributes.insert(
            "nested".to_owned(),
            json!([{"deep": ["a", "b", {"value": true}]}]),
        );
        let mut extraction = Extraction {
            nodes: vec![compass_languages::RawNodeRecord {
                id: "node".to_owned(),
                attributes,
            }],
            ..Extraction::default()
        };
        extraction.raw_calls = Some(vec![compass_languages::RawCall {
            caller_nid: "node".to_owned(),
            callee: "callee".to_owned(),
            is_member_call: None,
            source_file: "source.go".to_owned(),
            source_location: "1:1".to_owned(),
            receiver: None,
            receiver_type: None,
            lang: Some("go".to_owned()),
            extensions: Map::new(),
        }]);
        let before = serde_json::to_value(&extraction)?;

        compact_extraction(&mut extraction);

        assert_eq!(before, serde_json::to_value(extraction)?);
        Ok(())
    }

    #[test]
    fn compact_extraction_shrinks_only_materially_overallocated_vectors() {
        let mut overallocated_calls = Vec::with_capacity(64);
        overallocated_calls.push(compass_languages::RawCall {
            caller_nid: "node".to_owned(),
            callee: "callee".to_owned(),
            is_member_call: None,
            source_file: "source.go".to_owned(),
            source_location: "1:1".to_owned(),
            receiver: None,
            receiver_type: None,
            lang: Some("go".to_owned()),
            extensions: Map::new(),
        });
        let overallocated_capacity = overallocated_calls.capacity();

        let mut tight_nodes = Vec::with_capacity(2);
        tight_nodes.extend([
            RawNodeRecord {
                id: "node-1".to_owned(),
                attributes: Map::new(),
            },
            RawNodeRecord {
                id: "node-2".to_owned(),
                attributes: Map::new(),
            },
        ]);
        let tight_capacity = tight_nodes.capacity();

        let mut extraction = Extraction {
            nodes: tight_nodes,
            raw_calls: Some(overallocated_calls),
            ..Extraction::default()
        };
        compact_extraction(&mut extraction);

        assert!(
            extraction
                .raw_calls
                .as_ref()
                .is_some_and(|calls| calls.capacity() < overallocated_capacity)
        );
        assert_eq!(extraction.nodes.capacity(), tight_capacity);
    }

    #[test]
    fn ast_fact_digest_is_independent_of_map_insertion_order() -> Result<(), Box<dyn Error>> {
        let mut first_attributes = Map::new();
        first_attributes.insert("zeta".to_owned(), json!({"b": 2, "a": 1}));
        first_attributes.insert("alpha".to_owned(), json!(true));
        let mut second_attributes = Map::new();
        second_attributes.insert("alpha".to_owned(), json!(true));
        second_attributes.insert("zeta".to_owned(), json!({"a": 1, "b": 2}));
        let first = Extraction {
            nodes: vec![RawNodeRecord {
                id: "node".to_owned(),
                attributes: first_attributes,
            }],
            edges: vec![RawEdgeRecord {
                source: "node".to_owned(),
                target: "target".to_owned(),
                attributes: Map::from_iter([
                    ("zeta".to_owned(), json!({"b": 2, "a": 1})),
                    ("alpha".to_owned(), json!(true)),
                ]),
            }],
            ..Extraction::default()
        };
        let second = Extraction {
            nodes: vec![RawNodeRecord {
                id: "node".to_owned(),
                attributes: second_attributes,
            }],
            edges: vec![RawEdgeRecord {
                source: "node".to_owned(),
                target: "target".to_owned(),
                attributes: Map::from_iter([
                    ("alpha".to_owned(), json!(true)),
                    ("zeta".to_owned(), json!({"a": 1, "b": 2})),
                ]),
            }],
            ..Extraction::default()
        };

        assert_ne!(serde_json::to_vec(&first)?, serde_json::to_vec(&second)?);
        assert_eq!(
            extraction_fact_digest(&first)?,
            extraction_fact_digest(&second)?
        );
        Ok(())
    }

    #[test]
    fn ast_fact_digest_is_independent_of_fact_sequence_order() -> Result<(), Box<dyn Error>> {
        let first = Extraction {
            nodes: vec![
                RawNodeRecord {
                    id: "node-a".to_owned(),
                    attributes: Map::from_iter([("symbol_kind".to_owned(), json!("function"))]),
                },
                RawNodeRecord {
                    id: "node-b".to_owned(),
                    attributes: Map::from_iter([("symbol_kind".to_owned(), json!("class"))]),
                },
            ],
            edges: vec![
                RawEdgeRecord {
                    source: "node-a".to_owned(),
                    target: "node-b".to_owned(),
                    attributes: Map::from_iter([("kind".to_owned(), json!("calls"))]),
                },
                RawEdgeRecord {
                    source: "node-b".to_owned(),
                    target: "node-a".to_owned(),
                    attributes: Map::from_iter([("kind".to_owned(), json!("references"))]),
                },
            ],
            ..Extraction::default()
        };
        let mut second = first.clone();
        second.nodes.reverse();
        second.edges.reverse();

        assert_ne!(serde_json::to_vec(&first)?, serde_json::to_vec(&second)?);
        assert_eq!(
            extraction_fact_digest(&first)?,
            extraction_fact_digest(&second)?
        );
        Ok(())
    }

    #[test]
    fn ast_fact_digest_normalizes_cache_null_call_fields() -> Result<(), Box<dyn Error>> {
        let call = RawCall {
            caller_nid: "caller".to_owned(),
            callee: "callee".to_owned(),
            is_member_call: Some(false),
            source_file: "main.js".to_owned(),
            source_location: "L1".to_owned(),
            receiver: None,
            receiver_type: None,
            lang: Some("javascript".to_owned()),
            extensions: Map::new(),
        };
        let mut explicit_null = call.clone();
        explicit_null.receiver = Some(None);
        explicit_null.receiver_type = Some(None);
        let first = Extraction {
            raw_calls: Some(vec![call]),
            ..Extraction::default()
        };
        let second = Extraction {
            raw_calls: Some(vec![explicit_null]),
            ..Extraction::default()
        };

        assert_ne!(serde_json::to_vec(&first)?, serde_json::to_vec(&second)?);
        assert_eq!(
            extraction_fact_digest(&first)?,
            extraction_fact_digest(&second)?
        );
        Ok(())
    }

    #[test]
    fn graph_delta_change_count_walks_canonical_records() {
        let previous = [("a", 1_u8), ("c", 1_u8)];
        let current = [("a", 1_u8), ("b", 1_u8), ("c", 2_u8)];
        assert_eq!(
            changed_record_count(&previous, &current, |record| record.0),
            2
        );
        assert_eq!(
            changed_record_count(&current, &previous, |record| record.0),
            2
        );
    }

    #[test]
    fn graph_delta_candidate_falls_back_for_unsorted_records() {
        let build = compass_model::code_graph::BuildMetadata {
            builder_version: "test".to_owned(),
            schema_fingerprint: "schema".to_owned(),
            source_tree_digest: "source".to_owned(),
            configuration_digest: "configuration".to_owned(),
            generation_id: "generation".to_owned(),
            source_commit: None,
        };
        let node = |id: &str| compass_model::code_graph::NodeRecord {
            id: id.to_owned(),
            kind: compass_model::code_graph::NodeKind::Function,
            roles: Vec::new(),
            name: id.to_owned(),
            qualified_name: id.to_owned(),
            language: Some("rust".to_owned()),
            framework: None,
            source: None,
            details: None,
            evidence: Vec::new(),
            coverage: Vec::new(),
            diagnostics: Vec::new(),
            community: None,
        };
        let mut previous = V1GraphDocument::empty_v1(build);
        previous.nodes = vec![node("a"), node("b")];
        let mut current = previous.clone();
        current.nodes[0].name = "changed".to_owned();
        assert!(graph_delta_candidate(&previous, &current));

        let mut unsorted_previous = previous.clone();
        unsorted_previous.nodes.reverse();
        assert!(!graph_delta_candidate(&unsorted_previous, &current));

        let mut unsorted_current = current;
        unsorted_current.nodes.reverse();
        assert!(!graph_delta_candidate(&previous, &unsorted_current));
    }

    #[test]
    fn previous_communities_scans_typed_and_legacy_nodes_without_loading_edges()
    -> Result<(), Box<dyn Error>> {
        let directory = tempfile::tempdir()?;
        let graph_path = directory.path().join("graph.json");
        fs::write(
            &graph_path,
            serde_json::to_vec(&json!({
                "directed": true,
                "multigraph": true,
                "graph": {},
                "nodes": [
                    {"id": "typed", "community": {"id": 4, "label": "typed"}},
                    {"id": "legacy", "community": 9},
                    {"id": "unclustered", "name": "No community"}
                ],
                "links": [{"id": "ignored-edge", "source": "typed", "target": "legacy"}]
            }))?,
        )?;

        let communities = previous_communities(&graph_path);
        assert_eq!(communities.get("typed"), Some(&4));
        assert_eq!(communities.get("legacy"), Some(&9));
        assert!(!communities.contains_key("unclustered"));
        Ok(())
    }

    #[test]
    fn prior_semantic_preservation_requires_an_owned_marker() -> Result<(), Box<dyn Error>> {
        let directory = tempfile::tempdir()?;
        assert!(!prior_semantic_layer_required(true, directory.path()));
        fs::write(directory.path().join(SEMANTIC_MARKER_FILE), b"{}")?;
        assert!(prior_semantic_layer_required(true, directory.path()));
        assert!(!prior_semantic_layer_required(false, directory.path()));
        Ok(())
    }

    #[test]
    fn nonempty_zero_token_semantic_layers_publish_a_marker() -> Result<(), Box<dyn Error>> {
        let directory = tempfile::tempdir()?;
        let semantic = SemanticLayer {
            fragment: json!({
                "nodes": [{"id": "semantic", "label": "Semantic", "file_type": "concept"}],
                "edges": [],
                "hyperedges": [],
                "input_tokens": 0,
                "output_tokens": 0,
                "failed_chunks": 0,
            }),
            refreshed_files: Vec::new(),
            partial_files: Vec::new(),
            allow_partial: false,
        };

        write_semantic_marker(directory.path(), Some(&semantic))?;

        assert!(directory.path().join(SEMANTIC_MARKER_FILE).is_file());
        Ok(())
    }

    #[test]
    fn resolver_source_text_enforces_the_pre_read_byte_limit() -> Result<(), Box<dyn Error>> {
        let directory = tempfile::tempdir()?;
        let source = directory.path().join("oversized.py");
        fs::write(&source, b"0123456789")?;

        assert!(read_source_text_with_limit(&source, 9).is_none());
        assert_eq!(
            read_source_text_with_limit(&source, 10)
                .map(|(_, body)| body)
                .as_deref(),
            Some("0123456789")
        );
        Ok(())
    }

    #[test]
    fn universal_cache_entries_reject_any_replaced_raw_call_payload() -> Result<(), Box<dyn Error>>
    {
        let path = Path::new("cached.py");
        let mut extraction = compass_languages::Engine::default()
            .extract_source(path, b"def cached():\n    pass\n")?;
        assert!(cached_universal_evidence_matches(&extraction, path));

        extraction.raw_calls = Some(Vec::new());
        assert!(!cached_universal_evidence_matches(&extraction, path));
        Ok(())
    }

    #[test]
    fn evidence_only_universal_extractions_are_cacheable_ast_facts() -> Result<(), Box<dyn Error>> {
        let extraction = compass_languages::Engine::default()
            .extract_source(Path::new("cached.go"), b"package cached\n\nfunc Run() {}\n")?;
        assert!(extraction.nodes.is_empty());
        assert!(extraction.edges.is_empty());
        assert!(extraction.semantic_evidence.is_some());
        assert!(extraction_has_cacheable_ast_facts(&extraction));
        assert!(!extraction_has_cacheable_ast_facts(&Extraction::default()));
        Ok(())
    }

    #[test]
    fn framework_cache_reuse_is_scoped_to_project_evidence() -> Result<(), Box<dyn Error>> {
        let directory = tempfile::tempdir()?;
        let source = directory.path().join("src/routes/+page.svelte");
        fs::create_dir_all(source.parent().ok_or("source has no parent")?)?;
        fs::write(&source, "<h1>Home</h1>")?;
        fs::write(
            directory.path().join("package.json"),
            r#"{"dependencies":{}}"#,
        )?;

        let initial = ProjectEvidenceIndex::build(directory.path(), std::slice::from_ref(&source));
        let mut extraction = Extraction::default();
        assert!(!cached_framework_evidence_matches(
            &extraction,
            &source,
            &initial
        ));
        extraction.extensions.insert(
            FRAMEWORK_PROJECT_EVIDENCE_EXTENSION.to_owned(),
            Value::String(initial.fingerprint_for(&source).to_owned()),
        );
        assert!(cached_framework_evidence_matches(
            &extraction,
            &source,
            &initial
        ));

        fs::write(
            directory.path().join("package.json"),
            r#"{"dependencies":{"@sveltejs/kit":"2.0.0"}}"#,
        )?;
        let changed = ProjectEvidenceIndex::build(directory.path(), std::slice::from_ref(&source));
        assert!(!cached_framework_evidence_matches(
            &extraction,
            &source,
            &changed
        ));
        Ok(())
    }

    #[test]
    fn universal_adapter_cache_requires_current_valid_evidence() -> Result<(), Box<dyn Error>> {
        let python = Path::new("src/example.py");
        let rust = Path::new("src/example.rs");
        assert!(!cached_universal_evidence_matches(
            &Extraction::default(),
            python
        ));
        assert!(!cached_universal_evidence_matches(
            &Extraction::default(),
            rust
        ));

        let source = b"def example():\n    return 1\n";
        let mut engine = Engine::default();
        let extracted = engine
            .extract_source_combined(python, "src/example.py", source)?
            .graph;
        assert!(cached_universal_evidence_matches(&extracted, python));
        let extracted_rust = engine
            .extract_source_combined(rust, "src/example.rs", b"fn example() {}\n")?
            .graph;
        assert!(cached_universal_evidence_matches(&extracted_rust, rust));

        let mut invalid = extracted;
        invalid
            .semantic_evidence
            .as_mut()
            .ok_or_else(|| std::io::Error::other("universal evidence is missing"))?
            .adapter
            .capabilities
            .clear();
        assert!(!cached_universal_evidence_matches(&invalid, python));
        Ok(())
    }

    #[test]
    fn precomputed_detection_cannot_cross_repository_roots() -> Result<(), Box<dyn Error>> {
        let detected_root = tempfile::tempdir()?;
        fs::write(
            detected_root.path().join("main.py"),
            "def main():\n    pass\n",
        )?;
        let detection = detect(detected_root.path(), &DetectOptions::default())?;

        let build_root = tempfile::tempdir()?;
        fs::write(
            build_root.path().join("main.py"),
            "def other():\n    pass\n",
        )?;
        let mut options = BuildOptions::new(build_root.path());
        options.precomputed_detection = Some(detection);

        let error = match build_local_graph(&options) {
            Ok(_) => return Err("mismatched detection unexpectedly succeeded".into()),
            Err(error) => error,
        };
        assert!(matches!(error, CoreError::DetectionRootMismatch));
        Ok(())
    }

    #[test]
    fn ast_id_remap_does_not_conflate_prefix_named_symbol_with_file() -> Result<(), Box<dyn Error>>
    {
        let directory = tempfile::tempdir()?;
        let root = directory.path();
        let source = root.join("internal/timeformattype_string.go");
        fs::create_dir_all(source.parent().ok_or("source path has no parent")?)?;
        fs::write(&source, "package internal\n\nfunc _() {}\n")?;
        let source_text = source.to_string_lossy().into_owned();
        let file_id = make_id(&[&source_text]);
        let prefix_symbol_id = make_id(&[&file_stem(&source)]);
        let mut extraction = Extraction {
            nodes: vec![
                RawNodeRecord {
                    id: file_id,
                    attributes: Map::from_iter([(
                        "source_file".to_owned(),
                        Value::String(source_text.clone()),
                    )]),
                },
                RawNodeRecord {
                    id: prefix_symbol_id.clone(),
                    attributes: Map::from_iter([(
                        "source_file".to_owned(),
                        Value::String(source_text),
                    )]),
                },
            ],
            edges: vec![RawEdgeRecord {
                source: "caller".to_owned(),
                target: prefix_symbol_id.clone(),
                attributes: Map::new(),
            }],
            ..Extraction::default()
        };
        let live_id_remap = ast_source_identity_map(std::slice::from_ref(&source), root);
        let id_remap =
            collect_ast_id_remap(std::slice::from_ref(&extraction), root, &live_id_remap);
        apply_ast_id_remap(
            &mut extraction,
            &id_remap,
            &format!("{}_", make_id(&[&root.to_string_lossy()])),
        );

        assert_eq!(extraction.nodes[0].id, "internal_timeformattype_string");
        assert_eq!(extraction.nodes[1].id, prefix_symbol_id);
        assert_eq!(extraction.edges[0].target, extraction.nodes[1].id);
        Ok(())
    }

    #[test]
    fn out_of_root_ast_sources_get_portable_ext_ids() -> Result<(), Box<dyn Error>> {
        let directory = tempfile::tempdir()?;
        let root = directory.path().join("WebApi");
        fs::create_dir(&root)?;
        let external = directory.path().join("Core/Core.csproj");
        let external_text = external.to_string_lossy().into_owned();
        let old_id = make_id(&[&external_text]);
        let source_id = "webapi".to_owned();
        let mut extraction = Extraction {
            nodes: vec![RawNodeRecord {
                id: old_id.clone(),
                attributes: Map::from_iter([(
                    "source_file".to_owned(),
                    Value::String(external_text),
                )]),
            }],
            edges: vec![RawEdgeRecord {
                source: source_id.clone(),
                target: old_id,
                attributes: Map::from_iter([(
                    "source_file".to_owned(),
                    Value::String(root.join("WebApi.csproj").to_string_lossy().into_owned()),
                )]),
            }],
            ..Extraction::default()
        };
        finalize_ast_extraction(&mut extraction, &root);
        assert_eq!(extraction.nodes[0].id, "ext_core_core_csproj");
        assert_eq!(
            extraction.nodes[0].string("source_file"),
            "../Core/Core.csproj"
        );
        assert_eq!(extraction.edges[0].target, "ext_core_core_csproj");
        assert_eq!(extraction.edges[0].source, source_id);
        Ok(())
    }

    #[test]
    fn portable_ast_cache_removes_worktree_relative_source_paths() -> Result<(), Box<dyn Error>> {
        let directory = tempfile::tempdir()?;
        let worktrees = directory.path().join("tmp");
        let root = worktrees.join("worktree-current/checkout");
        let source = root.join("fixtures/code-graph/routes/csharp/AspNetController.cs");
        fs::create_dir_all(source.parent().ok_or("source path has no parent")?)?;
        fs::write(&source, "class AspNetController {}\n")?;
        let escaped =
            "../../worktree-current/checkout/fixtures/code-graph/routes/csharp/AspNetController.cs";
        let mut extraction = Extraction {
            nodes: vec![RawNodeRecord {
                id: "controller".to_owned(),
                attributes: Map::from_iter([
                    ("source_file".to_owned(), Value::String(escaped.to_owned())),
                    ("origin_file".to_owned(), Value::String(escaped.to_owned())),
                ]),
            }],
            framework_facts: vec![
                serde_json::from_value(json!({
                    "type": "domain",
                    "fact": {
                        "framework": "aspnet",
                        "kind": "controller",
                        "name": "AspNetController",
                        "declaringScope": "AspNetController",
                        "anchor": {
                            "sourceFile": escaped,
                            "startByte": 0,
                            "endByte": 5,
                            "startLine": 1,
                            "startColumn": 0,
                            "endLine": 1,
                            "endColumn": 5
                        },
                        "origin": "ast"
                    }
                }))?,
                serde_json::from_value(json!({
                    "type": "annotation",
                    "fact": {
                        "packId": "spring-java",
                        "framework": "spring",
                        "annotationName": "Controller",
                        "ownerDeclarationId": "controller-declaration",
                        "ownerGraphNodeId": "controller",
                        "ownerQualifiedName": "AspNetController",
                        "ownerKind": "class",
                        "anchor": {
                            "sourceFile": escaped,
                            "startByte": 0,
                            "endByte": 5,
                            "startLine": 1,
                            "startColumn": 0,
                            "endLine": 1,
                            "endColumn": 5
                        }
                    }
                }))?,
            ],
            ..Extraction::default()
        };

        prepare_portable_ast_cache_entry(&mut extraction, &source, &root);

        let expected = "fixtures/code-graph/routes/csharp/AspNetController.cs";
        assert_eq!(extraction.nodes[0].string("source_file"), expected);
        assert_eq!(extraction.nodes[0].string("origin_file"), expected);
        assert!(extraction.framework_facts.iter().all(|fact| {
            let framework_source = match fact {
                RawFrameworkFact::Route(route) => &route.anchor.source_file,
                RawFrameworkFact::Domain(domain) => &domain.anchor.source_file,
                RawFrameworkFact::Annotation(annotation) => &annotation.anchor.source_file,
            };
            framework_source == expected
        }));
        Ok(())
    }

    #[test]
    fn astro_import_identities_do_not_include_checkout_root() -> Result<(), Box<dyn Error>> {
        let first = tempfile::tempdir()?;
        let second = tempfile::tempdir()?;
        let build = |root: &Path| -> Result<V1GraphDocument, Box<dyn Error>> {
            let source = root.join("src");
            fs::create_dir_all(&source)?;
            fs::create_dir_all(root.join(".hidden"))?;
            fs::write(
                source.join("Page.astro"),
                "---\nimport Layout from '../.hidden/Layout.astro';\nconst values: string[] = [];\n---\n<Layout />\n",
            )?;
            fs::write(root.join(".hidden/Layout.astro"), "<slot />\n")?;
            let mut options = BuildOptions::new(root);
            options.no_viz = true;
            let result = build_local_graph(&options)?;
            Ok(V1GraphDocument::load(
                &result.output_dir.join("graph.json"),
            )?)
        };
        let first_graph = build(first.path())?;
        let second_graph = build(second.path())?;
        let identities = |document: &V1GraphDocument| {
            let mut nodes = document
                .nodes
                .iter()
                .map(|node| node.id.clone())
                .collect::<Vec<_>>();
            let mut edges = document
                .links
                .iter()
                .map(|edge| {
                    (
                        edge.source.clone(),
                        edge.target.clone(),
                        edge.relation().to_owned(),
                    )
                })
                .collect::<Vec<_>>();
            nodes.sort();
            edges.sort();
            (nodes, edges)
        };
        assert_eq!(identities(&first_graph), identities(&second_graph));
        let encoded = first_graph
            .nodes
            .iter()
            .chain(&second_graph.nodes)
            .flat_map(|node| {
                [
                    node.id.clone(),
                    node.source_file().unwrap_or_default().to_owned(),
                ]
            })
            .chain(
                first_graph
                    .links
                    .iter()
                    .chain(&second_graph.links)
                    .flat_map(|edge| {
                        [
                            edge.source.clone(),
                            edge.target.clone(),
                            edge.source_file().unwrap_or_default().to_owned(),
                        ]
                    }),
            )
            .collect::<Vec<_>>()
            .join("\n");
        assert!(!encoded.contains(&make_id(&[&first.path().to_string_lossy()])));
        assert!(!encoded.contains(&make_id(&[&second.path().to_string_lossy()])));
        assert!(!encoded.contains(&first.path().to_string_lossy().to_string()));
        assert!(!encoded.contains(&second.path().to_string_lossy().to_string()));
        let punctuation_symbols = first_graph
            .nodes
            .iter()
            .filter(|node| node.label() == "[]")
            .collect::<Vec<_>>();
        assert!(
            punctuation_symbols.is_empty(),
            "generic punctuation symbols must not be guessed into the typed graph"
        );
        assert!(
            first_graph
                .graph
                .diagnostics
                .iter()
                .all(|diagnostic| diagnostic.code != "unresolved_node_kind"),
            "generic punctuation must be rejected during extraction instead of reaching publication: {:#?}",
            first_graph.graph.diagnostics
        );
        Ok(())
    }

    #[test]
    fn markdown_documents_edges_survive_portable_ast_remapping() -> Result<(), Box<dyn Error>> {
        let directory = tempfile::tempdir()?;
        fs::write(
            directory.path().join("guide.md"),
            "# Guide\n[Implementation](documented.rs)\n",
        )?;
        fs::write(
            directory.path().join("documented.rs"),
            "pub fn documented() {}\n",
        )?;
        let mut options = BuildOptions::new(directory.path());
        options.no_cluster = true;
        options.no_viz = true;
        options.force = true;
        let result = build_local_graph(&options)?;
        let graph = V1GraphDocument::load(&result.output_dir.join("graph.json"))?;
        assert!(
            graph
                .links
                .iter()
                .any(|edge| edge.kind.as_str() == "documents"),
            "graph={graph:#?}"
        );
        Ok(())
    }

    #[test]
    fn cold_warm_change_and_delete_builds_are_consistent() -> Result<(), Box<dyn Error>> {
        let directory = tempfile::tempdir()?;
        let root = directory.path();
        fs::write(
            root.join("main.py"),
            "from helper import work\n\ndef main():\n    return work()\n",
        )?;
        fs::write(root.join("helper.py"), "def work():\n    return 1\n")?;
        let mut options = BuildOptions::new(root);
        options.no_viz = true;
        options.max_workers = Some(2);

        let cold = build_local_graph(&options)?;
        assert_eq!(cold.files_considered, 2);
        assert_eq!(cold.files_extracted, 2);
        assert!(cold.timings.detect > Duration::ZERO);
        assert!(cold.timings.deterministic_extract > Duration::ZERO);
        assert!(cold.timings.graph_assembly > Duration::ZERO);
        assert!(cold.timings.publish > Duration::ZERO);
        assert!(cold.nodes > 0);
        assert!(cold.output_dir.join("graph.json").is_file());
        let overview_path = cold.output_dir.join("graph-overview.json");
        assert!(
            overview_path.is_file(),
            "clustered updates should prepare the VS Code graph overview even with --no-viz"
        );
        let overview: Value = serde_json::from_slice(&fs::read(&overview_path)?)?;
        assert_eq!(overview["schema"], "compass.graph-overview/2");
        assert_eq!(overview["nodeLimit"], 5_000);
        assert_eq!(
            overview["sourceGraphBytes"],
            fs::metadata(cold.output_dir.join("graph.json"))?.len()
        );
        assert_eq!(overview["model"]["schema"], "compass.viewer.graph/1");
        assert!(cold.output_dir.join("manifest.json").is_file());
        let cold_graph = V1GraphDocument::load(&cold.output_dir.join("graph.json"))?;
        let output_stats: Value =
            serde_json::from_slice(&fs::read(cold.output_dir.join("output-stats.json"))?)?;
        assert_eq!(output_stats["nodes"], cold_graph.nodes.len());
        assert_eq!(output_stats["edges"], cold_graph.links.len());
        assert_eq!(
            output_stats["graph_bytes"],
            fs::metadata(cold.output_dir.join("graph.json"))?.len()
        );
        assert!(cold_graph.nodes.iter().all(|node| {
            node.evidence
                .first()
                .is_some_and(|evidence| evidence.origin == EvidenceOrigin::Ast)
                && node
                    .source_file()
                    .is_none_or(|source| !Path::new(source).is_absolute())
        }));
        let cold_graph_bytes = fs::read(cold.output_dir.join("graph.json"))?;
        let cold_report_bytes = fs::read(cold.output_dir.join("GRAPH_REPORT.md"))?;

        let warm = build_local_graph(&options)?;
        assert_eq!(warm.files_extracted, 0);
        assert_eq!(warm.files_cached, 2);
        assert_eq!(warm.nodes, cold.nodes);
        assert_eq!(warm.edges, cold.edges);
        assert_eq!(
            fs::read(warm.output_dir.join("graph.json"))?,
            cold_graph_bytes
        );
        assert_eq!(
            fs::read(warm.output_dir.join("GRAPH_REPORT.md"))?,
            cold_report_bytes
        );

        fs::write(root.join("helper.py"), "def work():\n    return 2\n")?;
        let changed = build_local_graph(&options)?;
        assert_eq!(changed.files_extracted, 1);
        assert_eq!(changed.files_cached, 1);
        let changed_graph = V1GraphDocument::load(&changed.output_dir.join("graph.json"))?;
        let changed_graph_bytes = fs::read(changed.output_dir.join("graph.json"))?;
        assert_ne!(
            changed_graph_bytes, cold_graph_bytes,
            "a body-only edit must update definition hashes"
        );
        let identities = |document: &V1GraphDocument| {
            (
                document
                    .nodes
                    .iter()
                    .map(|node| node.id.clone())
                    .collect::<HashSet<_>>(),
                document
                    .links
                    .iter()
                    .map(|edge| (edge.source.clone(), edge.target.clone(), edge.relation()))
                    .collect::<HashSet<_>>(),
            )
        };
        assert_eq!(identities(&changed_graph), identities(&cold_graph));
        let implementation_hash = |document: &V1GraphDocument| {
            document
                .nodes
                .iter()
                .find(|node| node.label() == "work()")
                .and_then(|node| node.digest("implementation_hash"))
                .map(str::to_owned)
        };
        assert_ne!(
            implementation_hash(&changed_graph),
            implementation_hash(&cold_graph)
        );
        let warm_changed = build_local_graph(&options)?;
        assert_eq!(warm_changed.files_extracted, 0);
        assert_eq!(
            fs::read(warm_changed.output_dir.join("graph.json"))?,
            changed_graph_bytes,
            "cached and freshly extracted definition hashes must agree"
        );

        fs::remove_file(root.join("helper.py"))?;
        let deleted = build_local_graph(&options)?;
        assert_eq!(deleted.files_considered, 1);
        let graph = V1GraphDocument::load(&deleted.output_dir.join("graph.json"))?;
        assert!(
            graph
                .nodes
                .iter()
                .all(|node| node.source_file() != Some("helper.py"))
        );
        Ok(())
    }

    #[test]
    fn file_only_incremental_edit_reuses_store_index_roots() -> Result<(), Box<dyn Error>> {
        let directory = tempfile::tempdir()?;
        let root = directory.path();
        let source = root.join("main.py");
        fs::write(&source, "def main():\n    return 1\n")?;
        let mut options = BuildOptions::new(root);
        options.no_viz = true;

        let cold = build_local_graph(&options)?;
        let store_path = local_sqlite_store_path(&cold.output_dir.join("graph.json"));
        let before_store = SqliteStore::open(&store_path)?;
        let before_reader = GraphSnapshotReader::open_active(&before_store)?
            .ok_or("cold snapshot is not active")?;
        let before_roots = before_reader
            .manifest()
            .roots
            .iter()
            .map(|root| (root.index, root.digest.clone()))
            .collect::<BTreeMap<_, _>>();
        let before_graph = V1GraphDocument::load(&cold.output_dir.join("graph.json"))?;
        drop(before_reader);
        drop(before_store);

        fs::write(
            &source,
            "def main():\n    return 1\n\n# metadata-only edit\n",
        )?;
        let changed = build_local_graph(&options)?;
        let after_store = SqliteStore::open(&store_path)?;
        let after_reader = GraphSnapshotReader::open_active(&after_store)?
            .ok_or("changed snapshot is not active")?;
        let after_roots = after_reader
            .manifest()
            .roots
            .iter()
            .map(|root| (root.index, root.digest.clone()))
            .collect::<BTreeMap<_, _>>();
        let changed_graph = V1GraphDocument::load(&changed.output_dir.join("graph.json"))?;
        let file_changed = changed_graph
            .nodes
            .iter()
            .find(|node| node.kind == NodeKind::File)
            .ok_or("changed file node missing")?;
        assert!(
            before_graph
                .nodes
                .iter()
                .any(|node| { node.id == file_changed.id && node != file_changed })
        );
        for index in [
            IndexKind::Edges,
            IndexKind::Outgoing,
            IndexKind::Incoming,
            IndexKind::Files,
            IndexKind::Names,
            IndexKind::Terms,
            IndexKind::Communities,
            IndexKind::Diagnostics,
        ] {
            assert_eq!(
                before_roots.get(&index),
                after_roots.get(&index),
                "{index:?} root should be reused"
            );
        }
        assert_ne!(
            before_roots.get(&IndexKind::Nodes),
            after_roots.get(&IndexKind::Nodes)
        );
        assert_ne!(
            before_roots.get(&IndexKind::Metadata),
            after_roots.get(&IndexKind::Metadata)
        );
        Ok(())
    }

    #[test]
    fn fact_neutral_extract_incremental_publishes_only_file_changes() -> Result<(), Box<dyn Error>>
    {
        let directory = tempfile::tempdir()?;
        let root = directory.path();
        let source = root.join("main.py");
        fs::write(&source, "def main():\n    return 1\n")?;
        fs::create_dir_all(root.join("src"))?;
        let javascript = root.join("src/static.js");
        fs::write(
            &javascript,
            "function show(value) { return value; }\nmodule.exports = { show };\n",
        )?;
        let mut options = BuildOptions::new(root);
        options.no_cluster = true;
        options.no_viz = true;
        options.purpose = BuildPurpose::Extract;
        options.graph_storage = GraphStorage::Json;
        let empty_semantic = SemanticLayer {
            fragment: json!({"nodes": [], "edges": [], "hyperedges": []}),
            refreshed_files: Vec::new(),
            partial_files: Vec::new(),
            allow_partial: false,
        };

        let cold = build_graph_with_semantic(&options, &empty_semantic)?;
        assert!(cold.output_dir.join(AST_FACT_DIGESTS_FILE).is_file());
        let cold_graph = V1GraphDocument::load(&cold.output_dir.join("graph.json"))?;
        let cold_fact_state: AstFactDigestState =
            serde_json::from_slice(&fs::read(cold.output_dir.join(AST_FACT_DIGESTS_FILE))?)?;
        assert_eq!(cold_fact_state.schema, AST_FACT_DIGESTS_SCHEMA);
        assert_eq!(cold_fact_state.entries.len(), 2);

        fs::write(
            &source,
            "def main():\n    return 1\n\n# metadata-only edit\n",
        )?;
        let changed = build_graph_with_semantic(&options, &empty_semantic)?;
        assert_eq!(changed.files_extracted, 1);
        assert_eq!(changed.files_cached, 1);
        assert_eq!(changed.nodes, cold.nodes);
        assert_eq!(changed.edges, cold.edges);
        assert_eq!(changed.timings.graph_assembly, Duration::ZERO);

        let changed_graph = V1GraphDocument::load(&changed.output_dir.join("graph.json"))?;
        let mut canonical_changed = Vec::new();
        write_canonical_graph_json(&changed_graph, &mut canonical_changed)?;
        assert_eq!(
            fs::read(changed.output_dir.join("graph.json"))?,
            canonical_changed,
            "fact-neutral publication must remain byte-identical to canonical JSON"
        );
        let semantic_nodes = |graph: &V1GraphDocument| {
            graph
                .nodes
                .iter()
                .filter(|node| node.kind != NodeKind::File)
                .cloned()
                .collect::<Vec<_>>()
        };
        assert_eq!(semantic_nodes(&changed_graph), semantic_nodes(&cold_graph));
        assert_eq!(changed_graph.links, cold_graph.links);
        let cold_file = cold_graph
            .nodes
            .iter()
            .find(|node| node.kind == NodeKind::File && node.source_file() == Some("main.py"))
            .ok_or("cold file node missing")?;
        let cold_static_file = cold_graph
            .nodes
            .iter()
            .find(|node| node.kind == NodeKind::File && node.source_file() == Some("src/static.js"))
            .ok_or("cold static file node missing")?;
        let changed_file = changed_graph
            .nodes
            .iter()
            .find(|node| node.kind == NodeKind::File && node.source_file() == Some("main.py"))
            .ok_or("changed file node missing")?;
        let changed_static_file = changed_graph
            .nodes
            .iter()
            .find(|node| node.kind == NodeKind::File && node.source_file() == Some("src/static.js"))
            .ok_or("changed static file node missing")?;
        assert_eq!(changed_file.id, cold_file.id);
        assert_ne!(changed_file, cold_file);
        assert_ne!(changed_file.source, cold_file.source);
        assert_eq!(changed_static_file, cold_static_file);
        assert_ne!(changed_graph.graph.files, cold_graph.graph.files);

        fs::write(&source, "def main():\n    return 2\n\n# semantic edit\n")?;
        let semantic_change = build_graph_with_semantic(&options, &empty_semantic)?;
        let semantic_graph = V1GraphDocument::load(&semantic_change.output_dir.join("graph.json"))?;
        let implementation_hash = |graph: &V1GraphDocument| {
            graph
                .nodes
                .iter()
                .find(|node| node.label() == "main()")
                .and_then(|node| node.digest("implementation_hash"))
                .map(str::to_owned)
        };
        assert_ne!(
            implementation_hash(&semantic_graph),
            implementation_hash(&changed_graph)
        );
        Ok(())
    }

    #[test]
    fn clustered_fact_neutral_incremental_reuses_community_artifacts() -> Result<(), Box<dyn Error>>
    {
        let directory = tempfile::tempdir()?;
        let root = directory.path();
        let source = root.join("main.py");
        fs::write(
            &source,
            "def helper():\n    return 1\n\ndef main():\n    return helper()\n",
        )?;
        let mut options = BuildOptions::new(root);
        options.no_cluster = false;
        options.no_viz = true;
        options.purpose = BuildPurpose::Extract;
        options.graph_storage = GraphStorage::Json;
        let empty_semantic = SemanticLayer {
            fragment: json!({"nodes": [], "edges": [], "hyperedges": []}),
            refreshed_files: Vec::new(),
            partial_files: Vec::new(),
            allow_partial: false,
        };

        let cold = build_graph_with_semantic(&options, &empty_semantic)?;
        assert!(cold.communities > 0);
        let cold_analysis = fs::read(cold.output_dir.join("analysis.json"))?;
        let cold_graph = V1GraphDocument::load(&cold.output_dir.join("graph.json"))?;
        let cold_assignments = cold_graph
            .nodes
            .iter()
            .map(|node| (node.id.clone(), node.community.clone()))
            .collect::<BTreeMap<_, _>>();

        fs::write(
            &source,
            "def helper():\n    return 1\n\ndef main():\n    return helper()\n\n# metadata only\n",
        )?;
        let changed = build_graph_with_semantic(&options, &empty_semantic)?;
        let changed_graph = V1GraphDocument::load(&changed.output_dir.join("graph.json"))?;
        let changed_assignments = changed_graph
            .nodes
            .iter()
            .map(|node| (node.id.clone(), node.community.clone()))
            .collect::<BTreeMap<_, _>>();

        assert_eq!(changed.timings.graph_assembly, Duration::ZERO);
        assert_eq!(changed.communities, cold.communities);
        assert_eq!(changed_assignments, cold_assignments);
        assert_eq!(
            fs::read(changed.output_dir.join("analysis.json"))?,
            cold_analysis
        );
        Ok(())
    }

    #[test]
    fn fact_digest_ignores_python_file_envelope_edits() -> Result<(), Box<dyn Error>> {
        let directory = tempfile::tempdir()?;
        let source = directory.path().join("main.py");
        let initial_source = "def main():\n    return 1\n";
        let changed_source = "def main():\n    return 1\n\n# metadata-only edit\n";
        fs::write(&source, initial_source)?;
        let evidence = Arc::new(ProjectEvidenceIndex::build(
            directory.path(),
            std::slice::from_ref(&source),
        ));
        let mut engine = Engine::with_project_evidence(evidence);
        let initial =
            engine.extract_source_graph_only(&source, "main.py", initial_source.as_bytes())?;
        fs::write(&source, changed_source)?;
        let changed =
            engine.extract_source_graph_only(&source, "main.py", changed_source.as_bytes())?;
        let initial_digest = serde_json::to_value(FactDigestExtraction {
            extraction: &initial,
        })?;
        let changed_digest = serde_json::to_value(FactDigestExtraction {
            extraction: &changed,
        })?;
        assert_eq!(initial_digest, changed_digest);
        Ok(())
    }

    #[test]
    fn cached_cpp_declarations_are_not_project_merged_twice() -> Result<(), Box<dyn Error>> {
        let directory = tempfile::tempdir()?;
        let root = directory.path().join("checkout");
        fs::create_dir(&root)?;
        fs::write(
            root.join("arena.h"),
            r#"#include <cstddef>
class Arena {
 public:
  Arena();
  Arena(const Arena&) = delete;
  Arena& operator=(const Arena&) = delete;
  ~Arena();
  char* Allocate(size_t bytes);
  char* AllocateAligned(size_t bytes);
 private:
  char* AllocateFallback(size_t bytes);
};
inline char* Arena::Allocate(size_t bytes) {
  return AllocateFallback(bytes);
}
"#,
        )?;
        fs::write(
            root.join("arena.cc"),
            r#"#include "arena.h"
Arena::Arena() {}
Arena::~Arena() {}
char* Arena::AllocateFallback(size_t bytes) { return nullptr; }
char* Arena::AllocateAligned(size_t bytes) { return Allocate(bytes); }
"#,
        )?;
        let output = directory.path().join("output");
        let cache = directory.path().join("history-cache");
        let semantic = SemanticLayer {
            fragment: json!({
                "nodes": [],
                "edges": [],
                "hyperedges": [],
                "input_tokens": 0,
                "output_tokens": 0,
                "failed_chunks": 0,
            }),
            refreshed_files: Vec::new(),
            partial_files: Vec::new(),
            allow_partial: false,
        };
        let mut options = BuildOptions::new(&root);
        options.output_root = Some(output);
        options.cache_root = Some(cache);
        options.no_viz = true;
        options.program_analysis = true;
        options.purpose = BuildPurpose::Extract;

        let (cold_result, cold) = build_graph_with_layers_retained(&options, Some(&semantic), &[])?;
        assert_eq!(cold_result.files_extracted, 2);
        options.force = true;
        options.reuse_cache_on_force = true;
        let (warm_result, warm) = build_graph_with_layers_retained(&options, Some(&semantic), &[])?;
        assert_eq!(warm_result.files_cached, 2);
        assert_eq!(warm.document, cold.document);
        Ok(())
    }

    #[test]
    fn update_preserves_semantic_layer_but_replaces_ast_layer() -> Result<(), Box<dyn Error>> {
        let directory = tempfile::tempdir()?;
        let root = directory.path();
        fs::write(root.join("main.py"), "def before():\n    return 1\n")?;
        fs::write(root.join("domain.md"), "# Domain rule\n")?;
        let mut options = BuildOptions::new(root);
        options.no_viz = true;
        let semantic = SemanticLayer {
            fragment: json!({
                "nodes": [{
                    "id": "semantic_domain_rule",
                    "label": "Domain rule",
                    "file_type": "concept",
                    "source_file": "domain.md",
                }],
                "edges": [],
                "hyperedges": [],
                "failed_chunks": 0,
            }),
            refreshed_files: vec![PathBuf::from("domain.md")],
            partial_files: Vec::new(),
            allow_partial: false,
        };
        build_graph_with_semantic(&options, &semantic)?;

        fs::write(root.join("main.py"), "def after():\n    return 2\n")?;
        let updated = build_local_graph(&options)?;
        let graph = V1GraphDocument::load(&updated.output_dir.join("graph.json"))?;
        assert!(graph.nodes.iter().any(|node| node.label() == "Domain rule"));
        assert!(graph.nodes.iter().any(|node| node.label() == "after()"));
        assert!(!graph.nodes.iter().any(|node| node.label() == "before()"));
        Ok(())
    }

    #[test]
    fn update_does_not_duplicate_semantic_backed_documents() -> Result<(), Box<dyn Error>> {
        let directory = tempfile::tempdir()?;
        let root = directory.path();
        fs::write(root.join("guide.md"), "# Guide\n\nLocal structure.\n")?;
        let mut options = BuildOptions::new(root);
        options.no_viz = true;
        let semantic = SemanticLayer {
            fragment: json!({
                "nodes": [{
                    "id": "semantic_guide",
                    "label": "Guide concept",
                    "file_type": "concept",
                    "source_file": "guide.md",
                }],
                "edges": [],
                "hyperedges": [],
                "failed_chunks": 0,
            }),
            refreshed_files: vec![PathBuf::from("guide.md")],
            partial_files: Vec::new(),
            allow_partial: false,
        };
        build_graph_with_semantic(&options, &semantic)?;

        let updated = build_local_graph(&options)?;
        let graph = V1GraphDocument::load(&updated.output_dir.join("graph.json"))?;
        assert_eq!(
            graph
                .nodes
                .iter()
                .filter(|node| node.label() == "Guide concept")
                .count(),
            1
        );
        Ok(())
    }

    #[test]
    fn semantic_enrichment_preserves_markdown_and_html_structure() -> Result<(), Box<dyn Error>> {
        let directory = tempfile::tempdir()?;
        let root = directory.path();
        fs::write(root.join("guide.md"), "# Guide\n\n[Next](next.md)\n")?;
        fs::write(
            root.join("page.html"),
            "<main id=\"top\"><h1>Page</h1><p><a href=\"#top\">Top</a></p></main>\n",
        )?;
        let mut options = BuildOptions::new(root);
        options.no_viz = true;
        let semantic = SemanticLayer {
            fragment: json!({
                "nodes": [{
                    "id": "semantic_page",
                    "label": "Page concept",
                    "file_type": "concept",
                    "source_file": "page.html",
                }],
                "edges": [],
                "hyperedges": [],
                "failed_chunks": 0,
            }),
            refreshed_files: vec![PathBuf::from("guide.md"), PathBuf::from("page.html")],
            partial_files: Vec::new(),
            allow_partial: false,
        };
        let cold = build_local_graph(&options)?;
        let cold_graph = V1GraphDocument::load(&cold.output_dir.join("graph.json"))?;
        let structural_ids = cold_graph
            .nodes
            .iter()
            .filter(|node| matches!(node.source_file(), Some("guide.md" | "page.html")))
            .map(|node| node.id.clone())
            .collect::<std::collections::BTreeSet<_>>();
        assert!(!structural_ids.is_empty());

        let enriched = build_graph_with_semantic(&options, &semantic)?;
        let enriched_graph = V1GraphDocument::load(&enriched.output_dir.join("graph.json"))?;
        let enriched_ids = enriched_graph
            .nodes
            .iter()
            .map(|node| node.id.clone())
            .collect::<std::collections::BTreeSet<_>>();
        assert!(
            structural_ids.is_subset(&enriched_ids),
            "missing structural IDs: {:?}; enriched IDs: {:?}",
            structural_ids.difference(&enriched_ids).collect::<Vec<_>>(),
            enriched_ids
        );
        assert!(
            enriched_graph
                .nodes
                .iter()
                .any(|node| node.label() == "Page concept")
        );
        assert!(enriched_graph.links.iter().any(|edge| {
            edge.source_file() == Some("page.html") && edge.relation() == "contains"
        }));
        Ok(())
    }

    #[test]
    fn no_cluster_always_publishes_the_v1_contract() -> Result<(), Box<dyn Error>> {
        let extract_dir = tempfile::tempdir()?;
        fs::write(
            extract_dir.path().join("main.py"),
            "def main():\n    pass\n",
        )?;
        let mut extract = BuildOptions::new(extract_dir.path());
        extract.no_cluster = true;
        extract.purpose = BuildPurpose::Extract;
        let result = build_local_graph(&extract)?;
        let value: Value =
            serde_json::from_slice(&fs::read(result.output_dir.join("graph.json"))?)?;
        assert_eq!(value["graph"]["schema"], "compass.graph/1");
        assert_eq!(value["directed"], true);
        assert_eq!(value["multigraph"], true);
        assert!(value.get("links").is_some());
        assert!(value.get("edges").is_none());
        assert!(!result.output_dir.join("GRAPH_REPORT.md").exists());
        assert!(!result.output_dir.join("analysis.json").exists());

        let update_dir = tempfile::tempdir()?;
        fs::write(update_dir.path().join("main.py"), "def main():\n    pass\n")?;
        let mut update = BuildOptions::new(update_dir.path());
        update.no_cluster = true;
        let result = build_local_graph(&update)?;
        let bytes = fs::read(result.output_dir.join("graph.json"))?;
        let value: Value = serde_json::from_slice(&bytes)?;
        assert_eq!(value["graph"]["schema"], "compass.graph/1");
        assert_eq!(value["directed"], true);
        assert_eq!(value["multigraph"], true);
        assert!(value.get("links").is_some());
        assert!(value.get("edges").is_none());
        assert!(result.output_dir.join("source-root.txt").is_file());
        Ok(())
    }

    #[test]
    fn code_and_deterministic_document_sources_are_both_published() -> Result<(), Box<dyn Error>> {
        let directory = tempfile::tempdir()?;
        fs::write(
            directory.path().join("zzz.py"),
            "def code_symbol():\n    return 1\n",
        )?;
        fs::write(
            directory.path().join("aaa.md"),
            "# Document heading\n\nDocument body.\n",
        )?;
        let mut options = BuildOptions::new(directory.path());
        options.no_cluster = true;
        options.no_viz = true;

        let result = build_local_graph(&options)?;
        let graph = V1GraphDocument::load(&result.output_dir.join("graph.json"))?;
        let code_nodes = graph
            .nodes
            .iter()
            .filter(|node| node.source_file() == Some("zzz.py"))
            .collect::<Vec<_>>();
        let document_nodes = graph
            .nodes
            .iter()
            .filter(|node| node.source_file() == Some("aaa.md"))
            .collect::<Vec<_>>();

        assert!(!code_nodes.is_empty());
        assert!(!document_nodes.is_empty());
        Ok(())
    }

    #[test]
    fn unsupported_extensionless_shebang_is_skipped() -> Result<(), Box<dyn Error>> {
        let directory = tempfile::tempdir()?;
        fs::write(
            directory.path().join("main.py"),
            "def supported_symbol():\n    return 1\n",
        )?;
        fs::write(
            directory.path().join("vendor-treadmill"),
            "#!/usr/bin/fish\nfunction unsupported_symbol; echo 1; end\n",
        )?;
        let mut options = BuildOptions::new(directory.path());
        options.no_cluster = true;
        options.no_viz = true;

        let result = build_local_graph(&options)?;
        let graph = V1GraphDocument::load(&result.output_dir.join("graph.json"))?;
        assert!(
            graph
                .nodes
                .iter()
                .any(|node| node.label() == "supported_symbol()")
        );
        assert!(
            graph
                .nodes
                .iter()
                .all(|node| node.source_file() != Some("vendor-treadmill"))
        );
        Ok(())
    }

    #[test]
    fn unchanged_no_cluster_update_uses_manifest_without_loading_cache()
    -> Result<(), Box<dyn Error>> {
        let directory = tempfile::tempdir()?;
        fs::write(
            directory.path().join("main.py"),
            "def main():\n    return 1\n",
        )?;
        let mut options = BuildOptions::new(directory.path());
        options.no_cluster = true;
        options.no_viz = true;
        let cold = build_local_graph(&options)?;
        let graph_path = cold.output_dir.join("graph.json");
        let graph_bytes = fs::read(&graph_path)?;
        let manifest_bytes = fs::read(cold.output_dir.join("manifest.json"))?;
        fs::remove_dir_all(directory.path().join("compass-out").join("cache"))?;

        let warm = build_local_graph(&options)?;
        assert_eq!(warm.files_extracted, 0);
        assert_eq!(warm.files_cached, 1);
        assert_eq!(fs::read(warm.output_dir.join("graph.json"))?, graph_bytes);
        assert_eq!(
            fs::read(warm.output_dir.join("manifest.json"))?,
            manifest_bytes
        );
        Ok(())
    }

    #[test]
    fn ast_manifest_prunes_existing_files_removed_from_scope() -> Result<(), Box<dyn Error>> {
        let directory = tempfile::tempdir()?;
        let root = directory.path();
        fs::write(root.join("main.py"), "def main():\n    return 1\n")?;
        fs::create_dir(root.join("generated"))?;
        fs::write(
            root.join("generated/copied.py"),
            "def generated():\n    return 2\n",
        )?;
        let mut options = BuildOptions::new(root);
        options.no_cluster = true;
        options.no_viz = true;

        let initial = build_local_graph(&options)?;
        assert_eq!(initial.files_considered, 2);

        options.extra_excludes = vec!["generated/**".to_owned()];
        let scoped = build_local_graph(&options)?;
        assert_eq!(scoped.files_considered, 1);
        let manifest: Value =
            serde_json::from_slice(&fs::read(scoped.output_dir.join("manifest.json"))?)?;
        assert_eq!(manifest.as_object().map(serde_json::Map::len), Some(1));

        let unchanged = build_local_graph(&options)?;
        assert_eq!(unchanged.files_extracted, 0);
        assert_eq!(unchanged.files_cached, 1);
        assert!(!unchanged.outputs_changed);
        Ok(())
    }

    #[test]
    fn semantic_layer_replaces_owned_facts_and_stamps_manifest() -> Result<(), Box<dyn Error>> {
        let directory = tempfile::tempdir()?;
        let root = directory.path();
        fs::write(root.join("main.py"), "def main():\n    return 1\n")?;
        fs::write(root.join("diagram.png"), b"not-decoded-by-core")?;
        let mut options = BuildOptions::new(root);
        options.purpose = BuildPurpose::Extract;
        options.no_viz = true;
        let source = root.join("diagram.png");
        let first_layer = SemanticLayer {
            fragment: json!({
                "nodes": [{
                    "id": "old_concept",
                    "label": "Old concept",
                    "file_type": "concept",
                    "source_file": source,
                }],
                "edges": [],
                "hyperedges": [],
                "input_tokens": 13,
                "output_tokens": 7,
                "failed_chunks": 0,
            }),
            refreshed_files: vec![source.clone()],
            partial_files: Vec::new(),
            allow_partial: false,
        };
        let first = build_graph_with_semantic(&options, &first_layer)?;
        let graph_path = first.output_dir.join("graph.json");
        let graph = V1GraphDocument::load(&graph_path)?;
        assert!(graph.nodes.iter().any(|node| node.label() == "Old concept"));
        let manifest: Value =
            serde_json::from_slice(&fs::read(first.output_dir.join("manifest.json"))?)?;
        assert!(
            manifest["diagram.png"]["ast_hash"]
                .as_str()
                .is_some_and(|hash| !hash.is_empty())
        );
        assert!(
            manifest["diagram.png"]["semantic_hash"]
                .as_str()
                .is_some_and(|hash| !hash.is_empty())
        );
        let analysis: Value =
            serde_json::from_slice(&fs::read(first.output_dir.join("analysis.json"))?)?;
        assert_eq!(analysis["tokens"], json!({"input": 13, "output": 7}));

        let second_layer = SemanticLayer {
            fragment: json!({
                "nodes": [{
                    "id": "new_concept",
                    "label": "New concept",
                    "file_type": "concept",
                    "source_file": "diagram.png",
                }],
                "edges": [],
                "hyperedges": [],
                "input_tokens": 3,
                "output_tokens": 2,
                "failed_chunks": 0,
            }),
            refreshed_files: vec![source],
            partial_files: Vec::new(),
            allow_partial: false,
        };
        let second = build_graph_with_semantic(&options, &second_layer)?;
        let graph = V1GraphDocument::load(&second.output_dir.join("graph.json"))?;
        assert!(!graph.nodes.iter().any(|node| node.label() == "Old concept"));
        assert!(graph.nodes.iter().any(|node| node.label() == "New concept"));
        let Some(semantic) = graph
            .nodes
            .iter()
            .find(|node| node.label() == "New concept")
        else {
            return Err("new semantic node was not written".into());
        };
        assert_eq!(semantic.source_file(), Some("diagram.png"));
        assert!(node_has_origin(semantic, EvidenceOrigin::Heuristic));
        Ok(())
    }

    #[test]
    fn incomplete_raw_semantic_shrink_requires_explicit_override() -> Result<(), Box<dyn Error>> {
        let directory = tempfile::tempdir()?;
        let root = directory.path();
        fs::write(root.join("main.py"), "def main():\n    return 1\n")?;
        fs::write(root.join("diagram.png"), b"not-decoded-by-core")?;
        let mut options = BuildOptions::new(root);
        options.purpose = BuildPurpose::Extract;
        options.no_cluster = true;
        options.no_viz = true;
        let source = root.join("diagram.png");
        let complete = SemanticLayer {
            fragment: json!({
                "nodes": [
                    {"id":"concept_a", "label":"Concept A", "file_type":"concept", "source_file":"diagram.png"},
                    {"id":"concept_b", "label":"Concept B", "file_type":"concept", "source_file":"diagram.png"}
                ],
                "edges": [],
                "hyperedges": [],
                "input_tokens": 5,
                "output_tokens": 4,
                "failed_chunks": 0,
            }),
            refreshed_files: vec![source.clone()],
            partial_files: Vec::new(),
            allow_partial: false,
        };
        let first = build_graph_with_semantic(&options, &complete)?;
        let graph_path = first.output_dir.join("graph.json");
        let original = fs::read(&graph_path)?;
        let mut incomplete = SemanticLayer {
            fragment: json!({
                "nodes": [{"id":"concept_a", "label":"Concept A", "file_type":"concept", "source_file":"diagram.png"}],
                "edges": [],
                "hyperedges": [],
                "input_tokens": 2,
                "output_tokens": 1,
                "failed_chunks": 1,
            }),
            refreshed_files: vec![source],
            partial_files: vec![PathBuf::from("diagram.png")],
            allow_partial: false,
        };
        let error = match build_graph_with_semantic(&options, &incomplete) {
            Ok(_) => return Err("incomplete semantic shrink unexpectedly succeeded".into()),
            Err(error) => error,
        };
        assert!(matches!(error, CoreError::IncompleteSemanticShrink { .. }));
        assert_eq!(fs::read(&graph_path)?, original);

        incomplete.allow_partial = true;
        let partial = build_graph_with_semantic(&options, &incomplete)?;
        let graph = V1GraphDocument::load(&partial.output_dir.join("graph.json"))?;
        assert!(graph.nodes.iter().any(|node| node.label() == "Concept A"));
        assert!(!graph.nodes.iter().any(|node| node.label() == "Concept B"));
        let manifest: Value =
            serde_json::from_slice(&fs::read(partial.output_dir.join("manifest.json"))?)?;
        assert_eq!(manifest["diagram.png"]["semantic_hash"], "");
        Ok(())
    }

    #[test]
    fn complete_semantic_run_may_shrink_and_prunes_retired_sources() -> Result<(), Box<dyn Error>> {
        let directory = tempfile::tempdir()?;
        let root = directory.path();
        fs::write(root.join("main.py"), "def main():\n    return 1\n")?;
        let image = root.join("diagram.png");
        fs::write(&image, b"not-decoded-by-core")?;
        let mut options = BuildOptions::new(root);
        options.purpose = BuildPurpose::Extract;
        options.no_viz = true;
        let complete = SemanticLayer {
            fragment: json!({
                "nodes": [
                    {"id":"concept_a", "label":"Concept A", "file_type":"concept", "source_file":"diagram.png"},
                    {"id":"concept_b", "label":"Concept B", "file_type":"concept", "source_file":"diagram.png"}
                ],
                "edges": [],
                "hyperedges": [],
                "failed_chunks": 0,
            }),
            refreshed_files: vec![image.clone()],
            partial_files: Vec::new(),
            allow_partial: false,
        };
        build_graph_with_semantic(&options, &complete)?;

        let smaller = SemanticLayer {
            fragment: json!({
                "nodes": [{"id":"concept_a", "label":"Concept A", "file_type":"concept", "source_file":"diagram.png"}],
                "edges": [],
                "hyperedges": [],
                "failed_chunks": 0,
            }),
            refreshed_files: vec![image.clone()],
            partial_files: Vec::new(),
            allow_partial: false,
        };
        let smaller = build_graph_with_semantic(&options, &smaller)?;
        let graph = V1GraphDocument::load(&smaller.output_dir.join("graph.json"))?;
        assert!(graph.nodes.iter().any(|node| node.label() == "Concept A"));
        assert!(!graph.nodes.iter().any(|node| node.label() == "Concept B"));

        fs::remove_file(&image)?;
        let empty = SemanticLayer {
            fragment: json!({
                "nodes": [],
                "edges": [],
                "hyperedges": [],
                "failed_chunks": 0,
            }),
            refreshed_files: Vec::new(),
            partial_files: Vec::new(),
            allow_partial: false,
        };
        let empty = build_graph_with_semantic(&options, &empty)?;
        let graph = V1GraphDocument::load(&empty.output_dir.join("graph.json"))?;
        assert!(!graph.nodes.iter().any(|node| node.label() == "Concept A"));
        let manifest: Value =
            serde_json::from_slice(&fs::read(empty.output_dir.join("manifest.json"))?)?;
        assert!(manifest.get("diagram.png").is_none());
        Ok(())
    }
}
