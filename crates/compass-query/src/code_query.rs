use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::Instant;

use compass_graph::{
    GRAPH_SNAPSHOT_MAX_ITEMS, GRAPH_SNAPSHOT_MAX_OBJECTS, GRAPH_TERM_POSTING_CHUNK_ITEMS,
    GraphSnapshotReader, SnapshotReadLimits, TermPostingWork,
};
use compass_ir::ProgramBundle;
use compass_model::code_graph::{
    BuildMetadata, EdgeKind, EdgeRecord, FileRecord, GraphDocument, NodeKind, NodeRecord,
};
use compass_model::provenance::EvidenceConfidence;
use compass_model::query_contract::{
    CallRequest, CodeQueryOperation, CodeQueryResponse, DiscoveryScopeKind, ExploreRequest,
    ImpactRequest, NodeTrailRequest, QueryDiagnostic, QueryDiagnosticCode, QueryEdge, QueryFile,
    QueryNode, QueryPath, SearchHit, SearchRequest, discovery_scope_postings,
    normalize_query_symbol, query_edge_from_record, query_node_from_record,
    query_path_from_records,
};
use compass_model::validate_build_metadata_identity;
use compass_store::SqliteStore;
use rusqlite::{Connection, OptionalExtension, params};

use crate::cql::{QueryError, QueryErrorKind};
use crate::graph_engine::LocalStoreSnapshot;
use crate::index::QueryEngineKind;
use crate::join_program_evidence;
use crate::ranking::{rank_search_candidates, resolution_rank_is_strictly_better};
use crate::recall::{CandidateSource, RecallBudget, SearchCandidatePool};
use crate::source::{VerifiedSource, verified_source};
use crate::telemetry::QueryInstrumentation;
use crate::text::{canonical_query_token, query_recall_terms, strip_diacritics};

type GraphPath = (Vec<String>, Vec<String>);
type BoundedPathResult = (Option<GraphPath>, bool);
const MAX_CODE_QUERY_CANDIDATES: u32 = 256;
const MAX_RECALL_FUZZY_VARIANTS_PER_TERM: usize = 192;
const MAX_RECALL_FUZZY_VARIANTS_TOTAL: usize = 256;
const MIN_RECALL_CANDIDATES_BEFORE_FUZZY: usize = 4;
const SEARCH_QUERY_CACHE_CAPACITY: usize = 64;
const FUZZY_LOOKUP_CACHE_CAPACITY: usize = 512;

const ALL_EDGE_KINDS: &[EdgeKind] = &[
    EdgeKind::Contains,
    EdgeKind::Embeds,
    EdgeKind::Calls,
    EdgeKind::Imports,
    EdgeKind::Exports,
    EdgeKind::Extends,
    EdgeKind::Implements,
    EdgeKind::MixesIn,
    EdgeKind::References,
    EdgeKind::TypeOf,
    EdgeKind::Returns,
    EdgeKind::Instantiates,
    EdgeKind::Overrides,
    EdgeKind::Decorates,
    EdgeKind::RoutesTo,
    EdgeKind::Reads,
    EdgeKind::Writes,
    EdgeKind::Aliases,
    EdgeKind::Registers,
    EdgeKind::Handles,
    EdgeKind::Publishes,
    EdgeKind::Subscribes,
    EdgeKind::Produces,
    EdgeKind::Consumes,
    EdgeKind::Schedules,
    EdgeKind::Triggers,
    EdgeKind::Tests,
    EdgeKind::DependsOn,
    EdgeKind::Documents,
    EdgeKind::MapsTo,
    EdgeKind::Renders,
];

#[derive(Clone, Copy, Debug)]
pub(crate) enum StructuralOperandRole {
    CallersTarget,
    CalleesSource,
    ImpactTarget,
    TrailSource,
    TrailTarget,
}

impl StructuralOperandRole {
    const fn relation_probe(self) -> (bool, &'static [EdgeKind]) {
        match self {
            Self::CallersTarget => (true, &[EdgeKind::Calls, EdgeKind::RoutesTo]),
            Self::CalleesSource => (false, &[EdgeKind::Calls]),
            Self::ImpactTarget => (true, IMPACT_KINDS),
            Self::TrailSource => (false, ALL_EDGE_KINDS),
            Self::TrailTarget => (true, ALL_EDGE_KINDS),
        }
    }
}

pub(crate) struct CandidateAssembly {
    pub(crate) pool: SearchCandidatePool,
    pub(crate) truncated: bool,
    pub(crate) candidate_nodes_read: u64,
    pub(crate) postings_decoded: u64,
    pub(crate) relation_edges_examined: u64,
}

pub(crate) struct TermCandidateRead {
    pub(crate) nodes: Vec<NodeRecord>,
    pub(crate) matched_concepts: BTreeMap<String, BTreeSet<String>>,
    pub(crate) truncated: bool,
    pub(crate) node_ids_decoded: u64,
    pub(crate) chunks_decoded: u64,
}

pub(crate) struct RelationshipCandidateRead {
    pub(crate) source_ids: Vec<String>,
    pub(crate) truncated: bool,
    pub(crate) node_ids_decoded: u64,
    pub(crate) chunks_decoded: u64,
}

pub(crate) struct RelationshipTargetRead {
    pub(crate) target_ids: Vec<String>,
    pub(crate) truncated: bool,
    pub(crate) ids_decoded: u64,
}

pub(crate) struct SelectedOutgoingRead {
    pub(crate) records: Vec<EdgeRecord>,
    pub(crate) edge_ids: Vec<String>,
    pub(crate) truncated: bool,
    pub(crate) examined: usize,
}

pub(crate) struct CandidateAssemblyPolicy<'a> {
    pub(crate) max_candidates: usize,
    pub(crate) source_lookup_limit: usize,
    pub(crate) max_candidate_reads: usize,
    pub(crate) max_candidate_probes: usize,
    pub(crate) bounded_posting_work: bool,
    pub(crate) admit: &'a dyn Fn(&NodeRecord) -> bool,
    pub(crate) check: &'a mut dyn FnMut() -> Result<(), QueryError>,
}

struct CandidateReadBudget {
    remaining: usize,
    read: u64,
    probes_remaining: usize,
    probes: u64,
    truncated: bool,
}

impl CandidateReadBudget {
    const fn new(read_limit: usize, probe_limit: usize) -> Self {
        Self {
            remaining: read_limit,
            read: 0,
            probes_remaining: probe_limit,
            probes: 0,
            truncated: false,
        }
    }

    fn begin_probe(&mut self) -> bool {
        if self.probes_remaining == 0 {
            self.truncated = true;
            return false;
        }
        self.probes_remaining = self.probes_remaining.saturating_sub(1);
        self.probes = self.probes.saturating_add(1);
        true
    }

    fn lookup_limit(&self, desired: usize) -> usize {
        desired.min(self.remaining.saturating_sub(1))
    }

    fn record(&mut self, returned: usize, source_truncated: bool) {
        let examined = returned.saturating_add(usize::from(source_truncated));
        self.remaining = self.remaining.saturating_sub(examined);
        self.read = self
            .read
            .saturating_add(u64::try_from(examined).unwrap_or(u64::MAX));
        self.truncated |= source_truncated || self.remaining == 0;
    }

    fn record_additional_probes(&mut self, probes: u64) {
        let requested = usize::try_from(probes).unwrap_or(usize::MAX);
        let admitted = requested.min(self.probes_remaining);
        self.probes_remaining = self.probes_remaining.saturating_sub(admitted);
        self.probes = self
            .probes
            .saturating_add(u64::try_from(admitted).unwrap_or(u64::MAX));
        self.truncated |= admitted < requested;
    }

    fn record_exact_work(&mut self, examined: usize, source_truncated: bool) {
        let admitted = examined.min(self.remaining);
        self.remaining = self.remaining.saturating_sub(admitted);
        self.read = self
            .read
            .saturating_add(u64::try_from(admitted).unwrap_or(u64::MAX));
        self.truncated |= source_truncated || admitted < examined || self.remaining == 0;
    }
}

struct TraversalBudget {
    remaining_nodes: usize,
    remaining_edges: usize,
    nodes_expanded: u64,
    edges_expanded: u64,
}

impl TraversalBudget {
    fn new(limits: &compass_model::query_contract::CodeQueryLimits) -> Self {
        Self {
            remaining_nodes: usize::try_from(limits.max_nodes).unwrap_or(usize::MAX),
            remaining_edges: usize::try_from(limits.max_edges).unwrap_or(usize::MAX),
            nodes_expanded: 0,
            edges_expanded: 0,
        }
    }

    const fn can_start_pair(&self) -> bool {
        self.remaining_nodes > 0 && self.remaining_edges > 0
    }

    fn consume_node(&mut self) -> bool {
        if self.remaining_nodes == 0 {
            false
        } else {
            self.remaining_nodes -= 1;
            self.nodes_expanded = self.nodes_expanded.saturating_add(1);
            true
        }
    }

    fn consume_edge(&mut self) -> bool {
        if self.remaining_edges == 0 {
            false
        } else {
            self.remaining_edges -= 1;
            self.edges_expanded = self.edges_expanded.saturating_add(1);
            true
        }
    }

    fn record_work(&self, instrumentation: &mut QueryInstrumentation) {
        instrumentation.work.nodes_expanded = instrumentation
            .work
            .nodes_expanded
            .saturating_add(self.nodes_expanded);
        instrumentation.work.edges_expanded = instrumentation
            .work
            .edges_expanded
            .saturating_add(self.edges_expanded);
    }
}

const IMPACT_KINDS: &[EdgeKind] = &[
    EdgeKind::Calls,
    EdgeKind::RoutesTo,
    EdgeKind::Imports,
    EdgeKind::Exports,
    EdgeKind::References,
    EdgeKind::DependsOn,
    EdgeKind::Reads,
    EdgeKind::Writes,
    EdgeKind::Publishes,
    EdgeKind::Subscribes,
    EdgeKind::Produces,
    EdgeKind::Consumes,
    EdgeKind::Schedules,
    EdgeKind::Triggers,
    EdgeKind::MapsTo,
    EdgeKind::Renders,
];

pub struct CodeQueryEngine {
    pub(crate) backend: CodeGraphBackend,
    pub(crate) program: Option<ProgramBundle>,
    pub(crate) connection: Option<Connection>,
    pub(crate) graph_path: PathBuf,
    pub(crate) index_path: PathBuf,
    pub(crate) partial_graph_message: Option<String>,
    pub(crate) engine_kind: QueryEngineKind,
    pub(crate) graph_identity: String,
    pub(crate) build_generation_identity: String,
    pub(crate) search_query_cache: Mutex<SearchQueryCache>,
    pub(crate) fuzzy_lookup_cache: Mutex<FuzzyLookupCache>,
}

impl CodeQueryEngine {
    /// Return the build identity pinned to this engine's exact graph
    /// realization.
    pub fn build_metadata(&self) -> Result<BuildMetadata, QueryError> {
        let build = match &self.backend {
            CodeGraphBackend::Materialized { graph, .. } => Ok(graph.graph.build.clone()),
            CodeGraphBackend::Store(snapshot) => snapshot
                .reader()?
                .metadata_summary()
                .map(|metadata| metadata.graph.build)
                .map_err(snapshot_error),
        }?;
        validate_build_metadata_identity(&build).map_err(|error| {
            QueryError::new(
                QueryErrorKind::CorruptArtifact,
                "invalid_graph_build_identity",
                error.to_string(),
            )
        })?;
        Ok(build)
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct PreparedSearchQuery {
    pub(crate) terms: Vec<String>,
    pub(crate) ranking_terms: Vec<String>,
    pub(crate) fts_query: String,
}

#[derive(Debug)]
pub(crate) struct SearchQueryCache {
    entries: HashMap<String, PreparedSearchQuery>,
    order: VecDeque<String>,
    capacity: usize,
}

impl Default for SearchQueryCache {
    fn default() -> Self {
        Self {
            entries: HashMap::with_capacity(SEARCH_QUERY_CACHE_CAPACITY),
            order: VecDeque::with_capacity(SEARCH_QUERY_CACHE_CAPACITY),
            capacity: SEARCH_QUERY_CACHE_CAPACITY,
        }
    }
}

impl SearchQueryCache {
    fn get(&mut self, query: &str) -> Option<PreparedSearchQuery> {
        let prepared = self.entries.get(query)?.clone();
        self.order.retain(|cached| cached != query);
        self.order.push_back(query.to_owned());
        Some(prepared)
    }

    fn insert(&mut self, query: String, prepared: PreparedSearchQuery) {
        self.order.retain(|cached| cached != &query);
        while self.entries.len() >= self.capacity {
            let Some(expired) = self.order.pop_front() else {
                break;
            };
            self.entries.remove(&expired);
        }
        self.order.push_back(query.clone());
        self.entries.insert(query, prepared);
    }
}

pub(crate) type FuzzyLookupValue = (Vec<NodeRecord>, bool);

#[derive(Debug)]
pub(crate) struct FuzzyLookupCache {
    entries: HashMap<(String, usize), FuzzyLookupValue>,
    order: VecDeque<(String, usize)>,
    capacity: usize,
}

impl Default for FuzzyLookupCache {
    fn default() -> Self {
        Self {
            entries: HashMap::with_capacity(FUZZY_LOOKUP_CACHE_CAPACITY),
            order: VecDeque::with_capacity(FUZZY_LOOKUP_CACHE_CAPACITY),
            capacity: FUZZY_LOOKUP_CACHE_CAPACITY,
        }
    }
}

impl FuzzyLookupCache {
    fn get(&mut self, name: &str, limit: usize) -> Option<FuzzyLookupValue> {
        let key = (name.to_owned(), limit);
        let value = self.entries.get(&key)?.clone();
        self.order.retain(|cached| cached != &key);
        self.order.push_back(key);
        Some(value)
    }

    fn insert(&mut self, name: String, limit: usize, value: FuzzyLookupValue) {
        let key = (name, limit);
        self.order.retain(|cached| cached != &key);
        while self.entries.len() >= self.capacity {
            let Some(expired) = self.order.pop_front() else {
                break;
            };
            self.entries.remove(&expired);
        }
        self.order.push_back(key.clone());
        self.entries.insert(key, value);
    }
}

pub(crate) enum CodeGraphBackend {
    Materialized {
        graph: Box<GraphDocument>,
        adjacency: Box<CodeAdjacencyIndex>,
        lookup: Box<CodeLookupIndex>,
    },
    Store(Box<LocalStoreSnapshot>),
}

/// Request-scoped graph view. Store discovery pins one immutable reader so
/// selector verification and database setup happen once for the whole query.
pub(crate) enum PinnedDiscoveryBackend<'a> {
    Materialized {
        graph: &'a GraphDocument,
        adjacency: &'a CodeAdjacencyIndex,
        lookup: &'a CodeLookupIndex,
    },
    Store(Box<GraphSnapshotReader<'a, SqliteStore>>),
}

pub(crate) struct CodeLookupIndex {
    node_by_id: HashMap<String, usize>,
    nodes_by_normalized_name: HashMap<String, Vec<usize>>,
    operation_nodes_by_term: HashMap<String, Vec<usize>>,
    file_by_path: HashMap<String, usize>,
    scope_values: HashMap<(u8, String), Vec<String>>,
}

impl CodeLookupIndex {
    pub(crate) fn build(graph: &GraphDocument) -> Self {
        let mut lookup = Self {
            node_by_id: HashMap::with_capacity(graph.nodes.len()),
            nodes_by_normalized_name: HashMap::new(),
            operation_nodes_by_term: HashMap::new(),
            file_by_path: HashMap::with_capacity(graph.graph.files.len()),
            scope_values: HashMap::new(),
        };
        for (index, node) in graph.nodes.iter().enumerate() {
            lookup.node_by_id.insert(node.id.clone(), index);
            for name in [&node.name, &node.qualified_name] {
                lookup
                    .nodes_by_normalized_name
                    .entry(normalize_symbol(name))
                    .or_default()
                    .push(index);
            }
            if is_operation_role_declaration(node) {
                let mut terms = compass_model::search::identifier_search_terms(&node.name);
                terms.extend(compass_model::search::identifier_search_terms(
                    &node.qualified_name,
                ));
                for term in terms {
                    lookup
                        .operation_nodes_by_term
                        .entry(term)
                        .or_default()
                        .push(index);
                }
            }
            for (posting_kind, value, canonical) in discovery_scope_postings(node) {
                let Some(kind) = scope_kind_from_posting(&posting_kind) else {
                    continue;
                };
                lookup
                    .scope_values
                    .entry((scope_kind_rank(kind), value))
                    .or_default()
                    .push(canonical);
            }
        }
        for nodes in lookup.nodes_by_normalized_name.values_mut() {
            nodes.sort_by(|left, right| graph.nodes[*left].id.cmp(&graph.nodes[*right].id));
            nodes.dedup();
        }
        for nodes in lookup.operation_nodes_by_term.values_mut() {
            nodes.sort_by(|left, right| graph.nodes[*left].id.cmp(&graph.nodes[*right].id));
            nodes.dedup();
        }
        for values in lookup.scope_values.values_mut() {
            values.sort();
            values.dedup();
        }
        for (index, file) in graph.graph.files.iter().enumerate() {
            lookup.file_by_path.insert(file.path.clone(), index);
        }
        lookup
    }

    fn node_by_id(&self, id: &str) -> Option<usize> {
        self.node_by_id.get(id).copied()
    }

    fn nodes_by_normalized_name(&self, name: &str) -> &[usize] {
        self.nodes_by_normalized_name
            .get(name)
            .map_or(&[], Vec::as_slice)
    }

    fn operation_nodes_for_terms(&self, terms: &[String], limit: usize) -> (Vec<usize>, bool) {
        let mut nodes = BTreeSet::new();
        let mut truncated = false;
        for term in terms {
            for index in self.operation_nodes_by_term.get(term).into_iter().flatten() {
                nodes.insert(*index);
                if nodes.len() > limit {
                    nodes.pop_last();
                    truncated = true;
                }
            }
        }
        (nodes.into_iter().collect(), truncated)
    }

    fn file_by_path(&self, path: &str) -> Option<usize> {
        self.file_by_path.get(path).copied()
    }

    fn scope_values(&self, kind: DiscoveryScopeKind, value: &str) -> &[String] {
        self.scope_values
            .get(&(scope_kind_rank(kind), value.to_owned()))
            .map_or(&[], Vec::as_slice)
    }
}

pub(crate) const fn scope_kind_rank(kind: DiscoveryScopeKind) -> u8 {
    match kind {
        DiscoveryScopeKind::Community => 0,
        DiscoveryScopeKind::Source => 1,
        DiscoveryScopeKind::Package => 2,
        DiscoveryScopeKind::Node => 3,
    }
}

fn scope_kind_from_posting(posting: &str) -> Option<DiscoveryScopeKind> {
    match posting {
        "community-id" | "community-label" => Some(DiscoveryScopeKind::Community),
        "source" => Some(DiscoveryScopeKind::Source),
        "package" => Some(DiscoveryScopeKind::Package),
        "node-id" | "node-qname" => Some(DiscoveryScopeKind::Node),
        _ => None,
    }
}

pub(crate) struct CodeAdjacencyIndex {
    incident: HashMap<String, Vec<usize>>,
    trusted_incident: HashMap<String, Vec<usize>>,
    incoming: HashMap<String, HashMap<EdgeKind, Vec<usize>>>,
    trusted_incoming: HashMap<String, HashMap<EdgeKind, Vec<usize>>>,
    outgoing: HashMap<String, HashMap<EdgeKind, Vec<usize>>>,
    trusted_outgoing: HashMap<String, HashMap<EdgeKind, Vec<usize>>>,
    by_id: HashMap<String, usize>,
}

impl CodeAdjacencyIndex {
    pub(crate) fn build(graph: &GraphDocument) -> Self {
        let mut adjacency = Self {
            incident: HashMap::new(),
            trusted_incident: HashMap::new(),
            incoming: HashMap::new(),
            trusted_incoming: HashMap::new(),
            outgoing: HashMap::new(),
            trusted_outgoing: HashMap::new(),
            by_id: HashMap::with_capacity(graph.links.len()),
        };
        for (index, edge) in graph.links.iter().enumerate() {
            index_edge(&mut adjacency.incident, &edge.source, index);
            index_edge(&mut adjacency.incident, &edge.target, index);
            index_directional_edge(&mut adjacency.outgoing, &edge.source, edge.kind, index);
            index_directional_edge(&mut adjacency.incoming, &edge.target, edge.kind, index);
            if !is_heuristic(edge) {
                index_edge(&mut adjacency.trusted_incident, &edge.source, index);
                index_edge(&mut adjacency.trusted_incident, &edge.target, index);
                index_directional_edge(
                    &mut adjacency.trusted_outgoing,
                    &edge.source,
                    edge.kind,
                    index,
                );
                index_directional_edge(
                    &mut adjacency.trusted_incoming,
                    &edge.target,
                    edge.kind,
                    index,
                );
            }
            adjacency.by_id.insert(edge.id.clone(), index);
        }
        for incident in [&mut adjacency.incident, &mut adjacency.trusted_incident] {
            for edges in incident.values_mut() {
                sort_edge_indices(edges, graph);
                edges.dedup();
            }
        }
        for directional in [
            &mut adjacency.incoming,
            &mut adjacency.trusted_incoming,
            &mut adjacency.outgoing,
            &mut adjacency.trusted_outgoing,
        ] {
            for by_kind in directional.values_mut() {
                for edges in by_kind.values_mut() {
                    sort_edge_indices(edges, graph);
                }
            }
        }
        adjacency
    }

    fn incident(&self, node: &str, include_heuristic: bool) -> &[usize] {
        let index = if include_heuristic {
            &self.incident
        } else {
            &self.trusted_incident
        };
        index.get(node).map_or(&[], Vec::as_slice)
    }

    fn matching_bounded(
        &self,
        graph: &GraphDocument,
        node: &str,
        inbound: bool,
        kinds: &[EdgeKind],
        include_heuristic: bool,
        limit: usize,
    ) -> (Vec<usize>, bool, usize) {
        let directional = match (inbound, include_heuristic) {
            (true, true) => &self.incoming,
            (true, false) => &self.trusted_incoming,
            (false, true) => &self.outgoing,
            (false, false) => &self.trusted_outgoing,
        };
        let Some(by_kind) = directional.get(node) else {
            return (Vec::new(), false, 0);
        };
        let buckets = kinds
            .iter()
            .filter_map(|kind| by_kind.get(kind).map(Vec::as_slice))
            .collect::<Vec<_>>();
        let mut positions = vec![0_usize; buckets.len()];
        let mut edges = Vec::with_capacity(limit.min(1_024).saturating_add(1));
        let retained = limit.saturating_add(1);
        let mut examined = 0_usize;
        while edges.len() < retained {
            examined = examined.saturating_add(buckets.len());
            let next = buckets
                .iter()
                .enumerate()
                .filter_map(|(bucket, edges)| {
                    edges
                        .get(positions[bucket])
                        .copied()
                        .map(|edge| (bucket, edge))
                })
                .min_by(|(_, left), (_, right)| graph.links[*left].id.cmp(&graph.links[*right].id));
            let Some((bucket, edge)) = next else {
                break;
            };
            positions[bucket] += 1;
            edges.push(edge);
        }
        let truncated = edges.len() > limit;
        if truncated {
            edges.truncate(limit);
        }
        (edges, truncated, examined)
    }

    #[cfg(test)]
    fn matching(
        &self,
        graph: &GraphDocument,
        node: &str,
        inbound: bool,
        kinds: &[EdgeKind],
    ) -> Vec<usize> {
        let (edges, truncated, _) =
            self.matching_bounded(graph, node, inbound, kinds, true, usize::MAX);
        debug_assert!(!truncated);
        edges
    }

    fn by_id(&self, id: &str) -> Option<usize> {
        self.by_id.get(id).copied()
    }
}

impl CodeGraphBackend {
    pub(crate) fn pin_discovery(&self) -> Result<PinnedDiscoveryBackend<'_>, QueryError> {
        match self {
            Self::Materialized {
                graph,
                adjacency,
                lookup,
            } => Ok(PinnedDiscoveryBackend::Materialized {
                graph,
                adjacency,
                lookup,
            }),
            Self::Store(snapshot) => {
                Ok(PinnedDiscoveryBackend::Store(Box::new(snapshot.reader()?)))
            }
        }
    }

    pub(crate) fn node_by_id(&self, id: &str) -> Result<Option<NodeRecord>, QueryError> {
        match self {
            Self::Materialized { graph, lookup, .. } => Ok(lookup
                .node_by_id(id)
                .map(|index| graph.nodes[index].clone())),
            Self::Store(snapshot) => snapshot.reader()?.get_node(id).map_err(snapshot_error),
        }
    }

    fn edge_by_id(&self, id: &str) -> Result<Option<EdgeRecord>, QueryError> {
        match self {
            Self::Materialized {
                graph, adjacency, ..
            } => Ok(adjacency.by_id(id).map(|index| graph.links[index].clone())),
            Self::Store(snapshot) => snapshot.reader()?.get_edge(id).map_err(snapshot_error),
        }
    }

    pub(crate) fn nodes_by_normalized_name(
        &self,
        name: &str,
        limit: usize,
    ) -> Result<(Vec<NodeRecord>, bool), QueryError> {
        let name = normalize_symbol(name);
        match self {
            Self::Materialized { graph, lookup, .. } => {
                let retained = limit.saturating_add(1);
                let mut nodes = lookup
                    .nodes_by_normalized_name(&name)
                    .iter()
                    .take(retained)
                    .map(|index| graph.nodes[*index].clone())
                    .collect::<Vec<_>>();
                let truncated = nodes.len() > limit;
                if truncated {
                    nodes.truncate(limit);
                }
                Ok((nodes, truncated))
            }
            Self::Store(snapshot) => {
                let (mut nodes, truncated) = snapshot
                    .reader()?
                    .nodes_by_normalized_name(&name, snapshot_limits(limit.saturating_add(1))?)
                    .map_err(snapshot_error)?;
                let truncated = truncated || nodes.len() > limit;
                if nodes.len() > limit {
                    nodes.truncate(limit);
                }
                Ok((nodes, truncated))
            }
        }
    }

    pub(crate) fn matching_bounded(
        &self,
        node: &str,
        inbound: bool,
        kinds: &[EdgeKind],
        include_heuristic: bool,
        limit: usize,
    ) -> Result<(Vec<EdgeRecord>, bool), QueryError> {
        match self {
            Self::Materialized {
                graph, adjacency, ..
            } => {
                // Both backends bound the same canonical raw edge prefix and
                // apply the heuristic filter afterward. Otherwise a dense
                // heuristic prefix changes both results and truncation.
                let (indices, truncated, _) =
                    adjacency.matching_bounded(graph, node, inbound, kinds, true, limit);
                let mut edges = indices
                    .into_iter()
                    .map(|index| graph.links[index].clone())
                    .collect::<Vec<_>>();
                if !include_heuristic {
                    edges.retain(|edge| !is_heuristic(edge));
                }
                Ok((edges, truncated))
            }
            Self::Store(snapshot) => {
                let (mut edges, truncated) = snapshot
                    .reader()?
                    .adjacency_by_kinds(
                        node,
                        inbound,
                        kinds,
                        snapshot_limits(limit.saturating_add(1))?,
                    )
                    .map_err(snapshot_error)?;
                if !include_heuristic {
                    edges.retain(|edge| !is_heuristic(edge));
                }
                edges.sort_by(|left, right| left.id.cmp(&right.id));
                let truncated = truncated || edges.len() > limit;
                if edges.len() > limit {
                    edges.truncate(limit);
                }
                Ok((edges, truncated))
            }
        }
    }

    pub(crate) fn incident_bounded(
        &self,
        node: &str,
        include_heuristic: bool,
        limit: usize,
    ) -> Result<(Vec<EdgeRecord>, bool), QueryError> {
        match self {
            Self::Materialized {
                graph, adjacency, ..
            } => {
                let retained = limit.saturating_add(1);
                let mut edges = adjacency
                    .incident(node, include_heuristic)
                    .iter()
                    .take(retained)
                    .map(|index| graph.links[*index].clone())
                    .collect::<Vec<_>>();
                let truncated = edges.len() > limit;
                if truncated {
                    edges.truncate(limit);
                }
                Ok((edges, truncated))
            }
            Self::Store(snapshot) => {
                let (mut edges, truncated) = snapshot
                    .reader()?
                    .incident(node, snapshot_limits(limit.saturating_add(1))?)
                    .map_err(snapshot_error)?;
                if !include_heuristic {
                    edges.retain(|edge| !is_heuristic(edge));
                }
                let truncated = truncated || edges.len() > limit;
                if edges.len() > limit {
                    edges.truncate(limit);
                }
                Ok((edges, truncated))
            }
        }
    }

    fn file_by_path(&self, path: &str) -> Result<Option<FileRecord>, QueryError> {
        match self {
            Self::Materialized { graph, lookup, .. } => Ok(lookup
                .file_by_path(path)
                .map(|index| graph.graph.files[index].clone())),
            Self::Store(snapshot) => snapshot
                .reader()?
                .file_by_path(path)
                .map_err(snapshot_error),
        }
    }

    fn store_term_candidates(
        &self,
        terms: &[String],
        limit: usize,
        bounded_posting_work: bool,
    ) -> Result<Option<TermCandidateRead>, QueryError> {
        let Self::Store(snapshot) = self else {
            return Ok(None);
        };
        let reader = snapshot.reader()?;
        let (mut nodes, truncated, node_ids_decoded, chunks_decoded) = if bounded_posting_work {
            let (nodes, truncated, work) = reader
                .nodes_for_terms_bounded_work(terms, snapshot_limits(limit)?)
                .map_err(snapshot_error)?;
            (nodes, truncated, work.node_ids_decoded, work.chunks_decoded)
        } else {
            let recall_ceiling =
                usize::try_from(compass_model::query_contract::MAX_INDEXED_CANDIDATE_NODES_READ)
                    .unwrap_or(usize::MAX);
            let (nodes, truncated) = reader
                .nodes_for_terms(terms, snapshot_limits(recall_ceiling)?)
                .map_err(snapshot_error)?;
            let decoded = u64::try_from(nodes.len()).unwrap_or(u64::MAX);
            (nodes, truncated, decoded, 0)
        };
        let truncated = truncated || nodes.len() > limit;
        if nodes.len() > limit {
            nodes.truncate(limit);
        }
        let concepts = terms.iter().cloned().collect::<BTreeSet<_>>();
        let matched_concepts = nodes
            .iter()
            .map(|node| (node.id.clone(), concepts.clone()))
            .collect();
        Ok(Some(TermCandidateRead {
            nodes,
            matched_concepts,
            truncated,
            node_ids_decoded,
            chunks_decoded,
        }))
    }
}

impl PinnedDiscoveryBackend<'_> {
    pub(crate) fn node_by_id(&self, id: &str) -> Result<Option<NodeRecord>, QueryError> {
        match self {
            Self::Materialized { graph, lookup, .. } => Ok(lookup
                .node_by_id(id)
                .map(|index| graph.nodes[index].clone())),
            Self::Store(reader) => reader.get_node(id).map_err(snapshot_error),
        }
    }

    pub(crate) fn nodes_by_normalized_name(
        &self,
        name: &str,
        limit: usize,
    ) -> Result<(Vec<NodeRecord>, bool), QueryError> {
        let name = normalize_symbol(name);
        match self {
            Self::Materialized { graph, lookup, .. } => {
                let retained = limit.saturating_add(1);
                let mut nodes = lookup
                    .nodes_by_normalized_name(&name)
                    .iter()
                    .take(retained)
                    .map(|index| graph.nodes[*index].clone())
                    .collect::<Vec<_>>();
                let truncated = nodes.len() > limit;
                nodes.truncate(limit);
                Ok((nodes, truncated))
            }
            Self::Store(reader) => {
                let (mut nodes, truncated) = reader
                    .nodes_by_normalized_name(&name, snapshot_limits(limit.saturating_add(1))?)
                    .map_err(snapshot_error)?;
                let truncated = truncated || nodes.len() > limit;
                nodes.truncate(limit);
                Ok((nodes, truncated))
            }
        }
    }

    pub(crate) fn resolve_scope_values(
        &self,
        kind: DiscoveryScopeKind,
        value: &str,
        limit: usize,
    ) -> Result<(Vec<String>, bool), QueryError> {
        match self {
            Self::Materialized { lookup, .. } => {
                let retained = limit.saturating_add(1);
                let mut values = lookup
                    .scope_values(kind, value)
                    .iter()
                    .take(retained)
                    .cloned()
                    .collect::<Vec<_>>();
                let truncated = values.len() > limit;
                values.truncate(limit);
                Ok((values, truncated))
            }
            Self::Store(reader) => {
                let mut values = BTreeSet::new();
                let mut truncated = false;
                for posting_kind in snapshot_scope_kinds(kind) {
                    let (found, found_truncated) = reader
                        .resolve_scope_values(
                            posting_kind,
                            value,
                            snapshot_limits(limit.saturating_add(1))?,
                        )
                        .map_err(scope_snapshot_error)?;
                    truncated |= found_truncated;
                    for value in found {
                        values.insert(value);
                        if values.len() > limit {
                            truncated = true;
                            values.pop_last();
                        }
                    }
                }
                Ok((values.into_iter().collect(), truncated))
            }
        }
    }

    pub(crate) fn matching_bounded(
        &self,
        node: &str,
        inbound: bool,
        kinds: &[EdgeKind],
        include_heuristic: bool,
        limit: usize,
    ) -> Result<(Vec<EdgeRecord>, bool), QueryError> {
        let (edges, truncated, _) =
            self.matching_bounded_counted(node, inbound, kinds, include_heuristic, limit)?;
        Ok((edges, truncated))
    }

    pub(crate) fn matching_bounded_counted(
        &self,
        node: &str,
        inbound: bool,
        kinds: &[EdgeKind],
        include_heuristic: bool,
        limit: usize,
    ) -> Result<(Vec<EdgeRecord>, bool, usize), QueryError> {
        match self {
            Self::Materialized {
                graph, adjacency, ..
            } => {
                let (indices, truncated, _) =
                    adjacency.matching_bounded(graph, node, inbound, kinds, true, limit);
                let mut edges = indices
                    .into_iter()
                    .map(|index| graph.links[index].clone())
                    .collect::<Vec<_>>();
                let examined = edges.len().saturating_add(usize::from(truncated));
                if !include_heuristic {
                    edges.retain(|edge| !is_heuristic(edge));
                }
                Ok((edges, truncated, examined))
            }
            Self::Store(reader) => {
                let (mut edges, truncated) = reader
                    .adjacency_by_kinds(
                        node,
                        inbound,
                        kinds,
                        snapshot_limits(limit.saturating_add(1))?,
                    )
                    .map_err(snapshot_error)?;
                edges.sort_by(|left, right| left.id.cmp(&right.id));
                let truncated = truncated || edges.len() > limit;
                edges.truncate(limit);
                let examined = edges.len().saturating_add(usize::from(truncated));
                if !include_heuristic {
                    edges.retain(|edge| !is_heuristic(edge));
                }
                Ok((edges, truncated, examined))
            }
        }
    }

    pub(crate) fn incident_bounded(
        &self,
        node: &str,
        include_heuristic: bool,
        limit: usize,
    ) -> Result<(Vec<EdgeRecord>, bool), QueryError> {
        match self {
            Self::Materialized {
                graph, adjacency, ..
            } => {
                let retained = limit.saturating_add(1);
                let mut edges = adjacency
                    .incident(node, true)
                    .iter()
                    .take(retained)
                    .map(|index| graph.links[*index].clone())
                    .collect::<Vec<_>>();
                let truncated = edges.len() > limit;
                edges.truncate(limit);
                if !include_heuristic {
                    edges.retain(|edge| !is_heuristic(edge));
                }
                Ok((edges, truncated))
            }
            Self::Store(reader) => {
                let (mut edges, truncated) = reader
                    .incident(node, snapshot_limits(limit.saturating_add(1))?)
                    .map_err(snapshot_error)?;
                edges.sort_by(|left, right| left.id.cmp(&right.id));
                let truncated = truncated || edges.len() > limit;
                edges.truncate(limit);
                if !include_heuristic {
                    edges.retain(|edge| !is_heuristic(edge));
                }
                Ok((edges, truncated))
            }
        }
    }

    pub(crate) fn outgoing_within_nodes_bounded_work(
        &self,
        source: &str,
        selected_node_ids: &BTreeSet<String>,
        include_heuristic: bool,
        limit: usize,
    ) -> Result<SelectedOutgoingRead, QueryError> {
        match self {
            Self::Materialized {
                graph, adjacency, ..
            } => {
                let (indices, truncated, examined) =
                    adjacency.matching_bounded(graph, source, false, ALL_EDGE_KINDS, true, limit);
                let mut edges = indices
                    .into_iter()
                    .map(|index| graph.links[index].clone())
                    .filter(|edge| selected_node_ids.contains(&edge.target))
                    .collect::<Vec<_>>();
                if !include_heuristic {
                    edges.retain(|edge| !is_heuristic(edge));
                }
                Ok(SelectedOutgoingRead {
                    records: edges,
                    edge_ids: Vec::new(),
                    truncated,
                    examined: examined.min(limit),
                })
            }
            Self::Store(reader) => {
                let (edge_ids, truncated, examined) = reader
                    .outgoing_edge_ids_within_nodes_bounded_work(
                        source,
                        selected_node_ids,
                        snapshot_limits(limit)?,
                    )
                    .map_err(snapshot_error)?;
                Ok(SelectedOutgoingRead {
                    records: Vec::new(),
                    edge_ids,
                    truncated,
                    examined,
                })
            }
        }
    }

    pub(crate) fn edges_by_ids(
        &self,
        ids: &BTreeSet<String>,
    ) -> Result<Vec<EdgeRecord>, QueryError> {
        if ids.is_empty() {
            return Ok(Vec::new());
        }
        match self {
            Self::Materialized {
                graph, adjacency, ..
            } => ids
                .iter()
                .map(|id| {
                    adjacency
                        .by_id(id)
                        .map(|index| graph.links[index].clone())
                        .ok_or_else(|| {
                            QueryError::new(
                                QueryErrorKind::GraphInvariant,
                                "discovery_edge_missing",
                                format!("outgoing index references missing edge {id}"),
                            )
                        })
                })
                .collect(),
            Self::Store(reader) => reader
                .get_edges_by_ids_bounded_work(ids, snapshot_limits(ids.len())?)
                .map_err(snapshot_error),
        }
    }

    pub(crate) fn supports_identifier_subwords(&self) -> Result<bool, QueryError> {
        match self {
            Self::Materialized { .. } => Ok(true),
            Self::Store(reader) => reader
                .supports_identifier_subwords()
                .map_err(snapshot_error),
        }
    }

    pub(crate) fn supports_relationship_terms(&self) -> Result<bool, QueryError> {
        match self {
            Self::Materialized { .. } => Ok(true),
            Self::Store(reader) => reader.supports_relationship_terms().map_err(snapshot_error),
        }
    }

    pub(crate) fn operation_role_candidates(
        &self,
        terms: &[String],
        limit: usize,
    ) -> Result<Option<TermCandidateRead>, QueryError> {
        match self {
            Self::Materialized { graph, lookup, .. } => {
                let (indices, truncated) = lookup.operation_nodes_for_terms(terms, limit);
                let nodes = indices
                    .into_iter()
                    .map(|index| graph.nodes[index].clone())
                    .collect::<Vec<_>>();
                Ok(Some(TermCandidateRead {
                    node_ids_decoded: u64::try_from(nodes.len()).unwrap_or(u64::MAX),
                    nodes,
                    matched_concepts: BTreeMap::new(),
                    truncated,
                    chunks_decoded: 0,
                }))
            }
            Self::Store(reader) => {
                if !reader
                    .supports_operation_role_terms()
                    .map_err(snapshot_error)?
                {
                    return Ok(None);
                }
                let minimum = GRAPH_TERM_POSTING_CHUNK_ITEMS.saturating_mul(terms.len().max(1));
                let (mut nodes, truncated, work) = reader
                    .operation_role_nodes_for_terms_bounded_work(
                        terms,
                        snapshot_limits(limit.max(minimum))?,
                    )
                    .map_err(snapshot_error)?;
                let truncated = truncated || nodes.len() > limit;
                nodes.truncate(limit);
                Ok(Some(TermCandidateRead {
                    nodes,
                    matched_concepts: BTreeMap::new(),
                    truncated,
                    node_ids_decoded: work.node_ids_decoded,
                    chunks_decoded: work.chunks_decoded,
                }))
            }
        }
    }

    pub(crate) fn declaration_candidates(
        &self,
        terms: &[String],
        limit: usize,
    ) -> Result<Option<TermCandidateRead>, QueryError> {
        let Self::Store(reader) = self else {
            return Ok(None);
        };
        if !reader
            .supports_declaration_terms()
            .map_err(snapshot_error)?
        {
            return Ok(None);
        }
        let minimum = GRAPH_TERM_POSTING_CHUNK_ITEMS.saturating_mul(terms.len().max(1));
        let (mut nodes, truncated, work) = reader
            .declaration_nodes_for_terms_bounded_work(terms, snapshot_limits(limit.max(minimum))?)
            .map_err(snapshot_error)?;
        let truncated = truncated || nodes.len() > limit;
        nodes.truncate(limit);
        Ok(Some(TermCandidateRead {
            nodes,
            matched_concepts: BTreeMap::new(),
            truncated,
            node_ids_decoded: work.node_ids_decoded,
            chunks_decoded: work.chunks_decoded,
        }))
    }

    fn store_term_candidates(
        &self,
        concepts: &[String],
        limit: usize,
    ) -> Result<Option<TermCandidateRead>, QueryError> {
        let Self::Store(reader) = self else {
            return Ok(None);
        };
        let read_limits = snapshot_limits(limit.max(GRAPH_TERM_POSTING_CHUNK_ITEMS))?;
        let exact_read_limits = snapshot_limits(
            limit
                .checked_div(concepts.len().max(1))
                .unwrap_or(limit)
                .max(GRAPH_TERM_POSTING_CHUNK_ITEMS),
        )?;
        let (mut nodes, mut truncated, mut work) = if let [concept] = concepts {
            reader
                .nodes_for_exact_term_bounded_work(concept, exact_read_limits)
                .map_err(snapshot_error)?
        } else {
            let mut intersection = None::<BTreeSet<String>>;
            let mut exact_truncated = false;
            let mut exact_work = TermPostingWork::default();
            for concept in concepts {
                let (term_ids, term_truncated, term_work) = reader
                    .node_ids_for_exact_term_bounded_work(concept, exact_read_limits)
                    .map_err(snapshot_error)?;
                exact_truncated |= term_truncated;
                exact_work.chunks_decoded = exact_work
                    .chunks_decoded
                    .saturating_add(term_work.chunks_decoded);
                exact_work.node_ids_decoded = exact_work
                    .node_ids_decoded
                    .saturating_add(term_work.node_ids_decoded);
                let term_ids = term_ids.into_iter().collect::<BTreeSet<_>>();
                match &mut intersection {
                    Some(previous) => previous.retain(|id| term_ids.contains(id)),
                    None => intersection = Some(term_ids),
                }
                if intersection.as_ref().is_some_and(BTreeSet::is_empty) {
                    break;
                }
            }
            let ids = intersection.unwrap_or_default();
            let nodes = if ids.is_empty() {
                Vec::new()
            } else {
                reader
                    .get_nodes_by_ids_bounded_work(&ids, snapshot_limits(ids.len())?)
                    .map_err(snapshot_error)?
            };
            (nodes, exact_truncated, exact_work)
        };
        if nodes.is_empty() && !truncated {
            (nodes, truncated, work) = reader
                .nodes_for_terms_bounded_work(concepts, read_limits)
                .map_err(snapshot_error)?;
        }
        let truncated = truncated || nodes.len() > limit;
        nodes.truncate(limit);
        let matched = concepts.iter().cloned().collect::<BTreeSet<_>>();
        let matched_concepts = nodes
            .iter()
            .map(|node| (node.id.clone(), matched.clone()))
            .collect();
        Ok(Some(TermCandidateRead {
            nodes,
            matched_concepts,
            truncated,
            node_ids_decoded: work.node_ids_decoded,
            chunks_decoded: work.chunks_decoded,
        }))
    }

    fn store_relationship_sources(
        &self,
        concept: &str,
        limit: usize,
    ) -> Result<Option<RelationshipCandidateRead>, QueryError> {
        let Self::Store(reader) = self else {
            return Ok(None);
        };
        let read_limits = snapshot_limits(limit.max(GRAPH_TERM_POSTING_CHUNK_ITEMS))?;
        let (mut source_ids, truncated, work) = reader
            .source_ids_for_exact_relationship_term_bounded_work(concept, read_limits)
            .map_err(snapshot_error)?;
        let truncated = truncated || source_ids.len() > limit;
        source_ids.truncate(limit);
        Ok(Some(RelationshipCandidateRead {
            source_ids,
            truncated,
            node_ids_decoded: work.node_ids_decoded,
            chunks_decoded: work.chunks_decoded,
        }))
    }

    fn store_relationship_source_matches_term(
        &self,
        source_id: &str,
        concept: &str,
    ) -> Result<Option<bool>, QueryError> {
        let Self::Store(reader) = self else {
            return Ok(None);
        };
        reader
            .relationship_source_matches_term(source_id, concept)
            .map(Some)
            .map_err(snapshot_error)
    }

    fn store_relationship_targets(
        &self,
        source_id: &str,
        concepts: &BTreeSet<String>,
        limit: usize,
    ) -> Result<Option<RelationshipTargetRead>, QueryError> {
        let Self::Store(reader) = self else {
            return Ok(None);
        };
        let (mut target_ids, truncated, work) = reader
            .relationship_target_ids_for_source_terms_bounded_work(
                source_id,
                concepts,
                snapshot_limits(limit)?,
            )
            .map_err(snapshot_error)?;
        let truncated = truncated || target_ids.len() > limit;
        target_ids.truncate(limit);
        Ok(Some(RelationshipTargetRead {
            target_ids,
            truncated,
            ids_decoded: work.node_ids_decoded,
        }))
    }
}

fn snapshot_scope_kinds(kind: DiscoveryScopeKind) -> &'static [&'static str] {
    match kind {
        DiscoveryScopeKind::Community => &["community-id", "community-label"],
        DiscoveryScopeKind::Source => &["source"],
        DiscoveryScopeKind::Package => &["package"],
        DiscoveryScopeKind::Node => &["node-id", "node-qname"],
    }
}

fn snapshot_limits(max_items: usize) -> Result<SnapshotReadLimits, QueryError> {
    if max_items == 0 || max_items > GRAPH_SNAPSHOT_MAX_ITEMS {
        return Err(QueryError::new(
            QueryErrorKind::InvalidParameter,
            "code_query_limit_exceeded",
            format!("snapshot query item limit must be between 1 and {GRAPH_SNAPSHOT_MAX_ITEMS}"),
        ));
    }
    Ok(SnapshotReadLimits {
        max_items,
        max_bytes: 1024 * 1024 * 1024,
        max_objects: GRAPH_SNAPSHOT_MAX_OBJECTS,
        max_depth: 64,
    })
}

fn snapshot_error(error: compass_graph::SnapshotError) -> QueryError {
    QueryError::new(
        QueryErrorKind::CorruptArtifact,
        "store_graph_snapshot_failed",
        error.to_string(),
    )
}

fn scope_snapshot_error(error: compass_graph::SnapshotError) -> QueryError {
    if matches!(
        error,
        compass_graph::SnapshotError::CapabilityUnavailable(_)
    ) {
        QueryError::new(
            QueryErrorKind::UnsupportedSchema,
            "scope_index_unavailable",
            error.to_string(),
        )
    } else {
        snapshot_error(error)
    }
}

fn index_directional_edge(
    index: &mut HashMap<String, HashMap<EdgeKind, Vec<usize>>>,
    node: &str,
    kind: EdgeKind,
    edge: usize,
) {
    index
        .entry(node.to_owned())
        .or_default()
        .entry(kind)
        .or_default()
        .push(edge);
}

fn index_edge(index: &mut HashMap<String, Vec<usize>>, node: &str, edge: usize) {
    index.entry(node.to_owned()).or_default().push(edge);
}

fn sort_edge_indices(edges: &mut [usize], graph: &GraphDocument) {
    edges.sort_by(|left, right| graph.links[*left].id.cmp(&graph.links[*right].id));
}

impl CodeQueryEngine {
    pub fn search(&self, request: SearchRequest) -> Result<CodeQueryResponse, QueryError> {
        self.search_instrumented(request, &mut QueryInstrumentation::default())
    }

    pub(crate) fn search_instrumented(
        &self,
        request: SearchRequest,
        instrumentation: &mut QueryInstrumentation,
    ) -> Result<CodeQueryResponse, QueryError> {
        validate_limits(&request.limits)?;
        let recall_started = Instant::now();
        let prepared = self.prepare_search_query(&request.query)?;
        let terms = prepared.terms;
        let query = prepared.fts_query;
        let mut response =
            CodeQueryResponse::empty(CodeQueryOperation::Search, request.limits.clone());
        if query.is_empty() {
            response.diagnostics.push(QueryDiagnostic {
                code: QueryDiagnosticCode::NoMatch,
                message: "Search query contains no searchable terms".to_owned(),
                node_id: None,
                path: None,
            });
            instrumentation.recall += recall_started.elapsed();
            let execution_started = Instant::now();
            let response = self.finish_response(&mut response);
            instrumentation.execution += execution_started.elapsed();
            return response;
        }
        let candidate_limit = usize::try_from(request.limits.max_candidates).unwrap_or(usize::MAX);
        let admit = |_: &NodeRecord| true;
        let mut check = || Ok(());
        let assembly = self.assemble_search_candidates(
            &request.query,
            &terms,
            &query,
            CandidateAssemblyPolicy {
                max_candidates: candidate_limit,
                source_lookup_limit: candidate_limit,
                max_candidate_reads: usize::try_from(
                    compass_model::query_contract::MAX_INDEXED_CANDIDATE_NODES_READ,
                )
                .unwrap_or(usize::MAX),
                max_candidate_probes: usize::try_from(
                    compass_model::query_contract::MAX_INDEXED_CANDIDATE_PROBES,
                )
                .unwrap_or(usize::MAX),
                bounded_posting_work: false,
                admit: &admit,
                check: &mut check,
            },
            None,
            false,
        )?;
        instrumentation.work.candidates_read = instrumentation
            .work
            .candidates_read
            .saturating_add(assembly.candidate_nodes_read);
        instrumentation.work.postings_decoded = instrumentation
            .work
            .postings_decoded
            .saturating_add(assembly.postings_decoded);
        instrumentation.work.edges_expanded = instrumentation
            .work
            .edges_expanded
            .saturating_add(assembly.relation_edges_examined);
        response.truncated |= assembly.truncated;
        instrumentation.recall += recall_started.elapsed();

        if response.truncated {
            let candidate_label = if candidate_limit == 1 {
                "candidate"
            } else {
                "candidates"
            };
            response.diagnostics.push(QueryDiagnostic {
                code: QueryDiagnosticCode::BoundedTruncation,
                message: format!(
                    "Search candidate recall was limited to {candidate_limit} {candidate_label}"
                ),
                node_id: None,
                path: None,
            });
        }
        let candidates = assembly.pool.into_vec();
        let candidate_count = candidates.len();
        let max_nodes = usize::try_from(request.limits.max_nodes).unwrap_or(usize::MAX);
        let ranking_started = Instant::now();
        let ranked = rank_search_candidates(&request.query, &terms, candidates, max_nodes);
        instrumentation.ranking += ranking_started.elapsed();

        let execution_started = Instant::now();
        if candidate_count > max_nodes {
            response.truncated = true;
            response.diagnostics.push(QueryDiagnostic {
                code: QueryDiagnosticCode::BoundedTruncation,
                message: format!("Search results were limited to {max_nodes} nodes"),
                node_id: None,
                path: None,
            });
        }
        for result in ranked {
            let score = result.score;
            let id = result.node_id;
            let matched_fields = result.matched_fields;
            let node = result.node;
            response.results.push(SearchHit {
                node_id: id,
                score,
                matched_fields,
            });
            response.nodes.push(query_node(&node));
        }
        if response.results.is_empty() {
            response.diagnostics.push(QueryDiagnostic {
                code: QueryDiagnosticCode::NoMatch,
                message: format!("No symbol matched {:?}", request.query),
                node_id: None,
                path: None,
            });
        }
        let response = self.finish_response(&mut response);
        instrumentation.execution += execution_started.elapsed();
        response
    }

    pub(crate) fn assemble_search_candidates(
        &self,
        raw_query: &str,
        terms: &[String],
        fts_query: &str,
        policy: CandidateAssemblyPolicy<'_>,
        role: Option<StructuralOperandRole>,
        include_heuristic: bool,
    ) -> Result<CandidateAssembly, QueryError> {
        let candidate_limit = policy.max_candidates;
        let budget = RecallBudget {
            max_total_candidates: candidate_limit,
            max_per_source: candidate_limit,
            max_fuzzy_candidates: candidate_limit.min(16),
        };
        let mut pool = SearchCandidatePool::new(budget);
        let mut truncated = false;
        let mut candidate_work =
            CandidateReadBudget::new(policy.max_candidate_reads, policy.max_candidate_probes);
        let mut postings_decoded = 0_u64;
        let mut relation_edges_examined = 0_u64;

        (policy.check)()?;
        if candidate_work.remaining > 0
            && candidate_work.begin_probe()
            && let Some(node) = self.backend.node_by_id(raw_query)?
        {
            candidate_work.record(1, false);
            if (policy.admit)(&node) {
                let _ = pool.add(CandidateSource::ExactId, node);
            }
        } else if candidate_work.remaining == 0 {
            candidate_work.truncated = true;
        }

        (policy.check)()?;
        let normalized_name_query = normalize_symbol(raw_query);
        let exact_limit = candidate_work.lookup_limit(policy.source_lookup_limit);
        if exact_limit > 0 && candidate_work.begin_probe() {
            let (exact_name_nodes, exact_name_truncated) = self
                .backend
                .nodes_by_normalized_name(&normalized_name_query, exact_limit)?;
            candidate_work.record(exact_name_nodes.len(), exact_name_truncated);
            add_admitted_candidates(
                &mut pool,
                CandidateSource::ExactName,
                exact_name_nodes,
                policy.admit,
            );
        } else if policy.source_lookup_limit > 0 {
            candidate_work.truncated = true;
        }

        for term in terms {
            (policy.check)()?;
            if term.chars().count() < 3 {
                continue;
            }
            let alias_limit = candidate_work.lookup_limit(policy.source_lookup_limit);
            if alias_limit == 0 || !candidate_work.begin_probe() {
                candidate_work.truncated = true;
                break;
            }
            let (alias_nodes, alias_truncated) =
                self.backend.nodes_by_normalized_name(term, alias_limit)?;
            candidate_work.record(alias_nodes.len(), alias_truncated);
            add_admitted_candidates(&mut pool, CandidateSource::Alias, alias_nodes, policy.admit);
        }

        (policy.check)()?;
        let term_limit = candidate_work.lookup_limit(policy.source_lookup_limit);
        if term_limit > 0 && candidate_work.begin_probe() {
            let term_read = if let Some(candidates) = self.backend.store_term_candidates(
                terms,
                term_limit,
                policy.bounded_posting_work,
            )? {
                candidates
            } else {
                let (nodes, truncated) =
                    self.materialized_term_candidates(fts_query, term_limit)?;
                let decoded = u64::try_from(nodes.len()).unwrap_or(u64::MAX);
                TermCandidateRead {
                    matched_concepts: nodes
                        .iter()
                        .map(|node| {
                            (
                                node.id.clone(),
                                terms.iter().cloned().collect::<BTreeSet<_>>(),
                            )
                        })
                        .collect(),
                    nodes,
                    truncated,
                    node_ids_decoded: decoded,
                    chunks_decoded: 0,
                }
            };
            postings_decoded = postings_decoded.saturating_add(term_read.node_ids_decoded);
            candidate_work.record_additional_probes(term_read.chunks_decoded);
            candidate_work.record_exact_work(
                usize::try_from(term_read.node_ids_decoded).unwrap_or(usize::MAX),
                term_read.truncated,
            );
            add_admitted_candidates(
                &mut pool,
                CandidateSource::TermIndex,
                term_read.nodes,
                policy.admit,
            );
        } else if !fts_query.is_empty() && policy.source_lookup_limit > 0 {
            candidate_work.truncated = true;
        }

        if pool.len() < candidate_limit.min(MIN_RECALL_CANDIDATES_BEFORE_FUZZY) {
            for variant in recall_fuzzy_term_variants(terms) {
                (policy.check)()?;
                if variant.len() < 3 {
                    continue;
                }
                if pool.len() >= candidate_limit {
                    break;
                }
                let fuzzy_limit = candidate_work
                    .lookup_limit(policy.source_lookup_limit.min(budget.max_fuzzy_candidates));
                if fuzzy_limit == 0 || !candidate_work.begin_probe() {
                    candidate_work.truncated = true;
                    break;
                }
                let (fuzzy_nodes, fuzzy_truncated) =
                    self.cached_fuzzy_nodes(&variant, fuzzy_limit)?;
                candidate_work.record(fuzzy_nodes.len(), fuzzy_truncated);
                add_admitted_candidates(
                    &mut pool,
                    CandidateSource::Fuzzy,
                    fuzzy_nodes,
                    policy.admit,
                );
                if pool.truncated_by_fuzzy_capacity() {
                    break;
                }
            }
        }

        if let Some(role) = role {
            let (inbound, kinds) = role.relation_probe();
            for node_id in pool.candidate_ids() {
                (policy.check)()?;
                let (edges, probe_truncated) = self.backend.matching_bounded(
                    &node_id,
                    inbound,
                    kinds,
                    include_heuristic,
                    1,
                )?;
                if !edges.is_empty() {
                    let _ = pool.tag(&node_id, CandidateSource::RelationSeed);
                }
                relation_edges_examined = relation_edges_examined
                    .saturating_add(u64::try_from(edges.len()).unwrap_or(u64::MAX));
                if probe_truncated {
                    relation_edges_examined = relation_edges_examined.saturating_add(1);
                }
            }
        }
        truncated |= candidate_work.truncated || pool.is_truncated();
        Ok(CandidateAssembly {
            pool,
            truncated,
            candidate_nodes_read: candidate_work.read,
            postings_decoded,
            relation_edges_examined,
        })
    }

    pub(crate) fn cached_fuzzy_nodes(
        &self,
        name: &str,
        limit: usize,
    ) -> Result<FuzzyLookupValue, QueryError> {
        {
            let mut cache = match self.fuzzy_lookup_cache.lock() {
                Ok(cache) => cache,
                Err(poisoned) => poisoned.into_inner(),
            };
            if let Some(value) = cache.get(name, limit) {
                return Ok(value);
            }
        }
        let value = self.backend.nodes_by_normalized_name(name, limit)?;
        let mut cache = match self.fuzzy_lookup_cache.lock() {
            Ok(cache) => cache,
            Err(poisoned) => poisoned.into_inner(),
        };
        cache.insert(name.to_owned(), limit, value.clone());
        Ok(value)
    }

    pub(crate) fn prepare_search_query(
        &self,
        query: &str,
    ) -> Result<PreparedSearchQuery, QueryError> {
        validate_search_query_size(query)?;
        {
            let mut cache = match self.search_query_cache.lock() {
                Ok(cache) => cache,
                Err(poisoned) => poisoned.into_inner(),
            };
            if let Some(prepared) = cache.get(query) {
                return Ok(prepared);
            }
        }

        // Validate the public lexical bound against the original query before
        // stop-word removal so many tiny tokens cannot bypass it.
        let literal_terms = search_query_terms(query)?;
        let terms = if query.chars().any(char::is_whitespace) {
            query_recall_terms(query)
                .into_iter()
                .filter(|term| {
                    !matches!(
                        term.to_ascii_uppercase().as_str(),
                        "AND" | "OR" | "NOT" | "NEAR"
                    )
                })
                .map(canonical_query_token)
                .collect::<BTreeSet<_>>()
                .into_iter()
                .collect::<Vec<_>>()
        } else {
            // A single symbol-shaped token may carry qualification and a
            // typo (`UserService.lits`). Preserve its literal pieces for the
            // bounded fuzzy resolver; natural-language phrases use canonical
            // stop-word and inflection handling above.
            literal_terms
        };
        validate_search_term_count(&terms)?;
        let prepared = PreparedSearchQuery {
            fts_query: fts_query_from_terms(&terms),
            ranking_terms: terms.clone(),
            terms,
        };
        let mut cache = match self.search_query_cache.lock() {
            Ok(cache) => cache,
            Err(poisoned) => poisoned.into_inner(),
        };
        cache.insert(query.to_owned(), prepared.clone());
        Ok(prepared)
    }

    pub(crate) fn prepare_discovery_query(
        &self,
        query: &str,
    ) -> Result<PreparedSearchQuery, QueryError> {
        validate_search_query_size(query)?;
        let cache_key = format!("discovery\0{query}");
        {
            let mut cache = match self.search_query_cache.lock() {
                Ok(cache) => cache,
                Err(poisoned) => poisoned.into_inner(),
            };
            if let Some(prepared) = cache.get(&cache_key) {
                return Ok(prepared);
            }
        }

        let recall_terms = query_recall_terms(query)
            .into_iter()
            .filter(|term| {
                !matches!(
                    term.to_ascii_uppercase().as_str(),
                    "AND" | "OR" | "NOT" | "NEAR"
                )
            })
            .take(compass_model::query_contract::MAX_INDEXED_QUERY_TERMS.saturating_add(1))
            .collect::<Vec<_>>();
        validate_search_term_count(&recall_terms)?;
        let ranking_terms = recall_terms
            .iter()
            .cloned()
            .map(canonical_query_token)
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect::<Vec<_>>();
        let mut terms = recall_terms.clone();
        for term in &ranking_terms {
            if terms.len() >= compass_model::query_contract::MAX_INDEXED_QUERY_TERMS {
                break;
            }
            if !terms.contains(term) {
                terms.push(term.clone());
            }
        }
        let prepared = PreparedSearchQuery {
            fts_query: fts_query_from_terms(&recall_terms),
            ranking_terms,
            terms,
        };
        let mut cache = match self.search_query_cache.lock() {
            Ok(cache) => cache,
            Err(poisoned) => poisoned.into_inner(),
        };
        cache.insert(cache_key, prepared.clone());
        Ok(prepared)
    }

    pub(crate) fn materialized_term_candidates(
        &self,
        query: &str,
        candidate_limit: usize,
    ) -> Result<(Vec<NodeRecord>, bool), QueryError> {
        let connection = self.connection.as_ref().ok_or_else(|| {
            QueryError::new(
                QueryErrorKind::Internal,
                "query_index_missing",
                "materialized query engine has no search index",
            )
        })?;
        let mut statement = connection
            .prepare(
                // Candidate truncation is part of the public response. Keep
                // it backend-neutral: immutable term postings and the JSON
                // FTS accelerator both select IDs in canonical byte order;
                // common Rust ranking is applied only after that bound.
                "SELECT n.id
                     FROM node_fts JOIN nodes n ON n.id = node_fts.node_id
                     WHERE node_fts MATCH ?1
                     ORDER BY n.id
                     LIMIT ?2",
            )
            .map_err(sql_error)?;
        let read_envelope = (candidate_limit.max(GRAPH_TERM_POSTING_CHUNK_ITEMS)
            / GRAPH_TERM_POSTING_CHUNK_ITEMS)
            .saturating_mul(GRAPH_TERM_POSTING_CHUNK_ITEMS);
        let sql_limit = i64::try_from(read_envelope).unwrap_or(i64::MAX);
        let mut rows = statement
            .query(params![query, sql_limit])
            .map_err(sql_error)?;
        let mut nodes = Vec::new();
        while let Some(row) = rows.next().map_err(sql_error)? {
            let id: String = row.get(0).map_err(sql_error)?;
            let node = self.backend.node_by_id(&id)?.ok_or_else(|| {
                QueryError::new(
                    QueryErrorKind::GraphInvariant,
                    "query_graph_invariant",
                    format!("index references absent graph node {id}"),
                )
            })?;
            nodes.push(node);
        }
        let truncated = if nodes.len() == read_envelope {
            let last = nodes.last().ok_or_else(|| {
                QueryError::new(
                    QueryErrorKind::Internal,
                    "term_posting_invariant",
                    "nonzero term posting envelope returned no rows",
                )
            })?;
            connection
                .query_row(
                    "SELECT EXISTS(
                       SELECT 1
                         FROM node_fts JOIN nodes n ON n.id = node_fts.node_id
                        WHERE node_fts MATCH ?1 AND n.id > ?2
                     )",
                    params![query, last.id],
                    |row| row.get::<_, bool>(0),
                )
                .map_err(sql_error)?
        } else {
            false
        };
        nodes.truncate(candidate_limit);
        Ok((nodes, truncated))
    }

    pub(crate) fn discovery_term_candidates(
        &self,
        backend: &PinnedDiscoveryBackend<'_>,
        concepts: &[String],
        candidate_limit: usize,
    ) -> Result<TermCandidateRead, QueryError> {
        if let Some(read) = backend.store_term_candidates(concepts, candidate_limit)? {
            return Ok(read);
        }
        let connection = self.connection.as_ref().ok_or_else(|| {
            QueryError::new(
                QueryErrorKind::Internal,
                "query_index_missing",
                "materialized query engine has no search index",
            )
        })?;
        let read_envelope = (candidate_limit.max(GRAPH_TERM_POSTING_CHUNK_ITEMS)
            / GRAPH_TERM_POSTING_CHUNK_ITEMS)
            .saturating_mul(GRAPH_TERM_POSTING_CHUNK_ITEMS);
        let sql_limit = i64::try_from(read_envelope).unwrap_or(i64::MAX);
        let mut nodes = Vec::new();
        let mut selected_query = String::new();
        let exact_query = concepts
            .iter()
            .map(|concept| format!("\"{}\"", concept.replace('"', "\"\"")))
            .collect::<Vec<_>>()
            .join(" AND ");
        let prefix_query = fts_query_from_terms(concepts);
        for query in [exact_query, prefix_query] {
            let mut statement = connection
                .prepare(
                    "SELECT n.id
                       FROM node_fts JOIN nodes n ON n.id = node_fts.node_id
                      WHERE node_fts MATCH ?1
                      ORDER BY n.id
                      LIMIT ?2",
                )
                .map_err(sql_error)?;
            let mut rows = statement
                .query(params![query, sql_limit])
                .map_err(sql_error)?;
            while let Some(row) = rows.next().map_err(sql_error)? {
                let id: String = row.get(0).map_err(sql_error)?;
                let node = backend.node_by_id(&id)?.ok_or_else(|| {
                    QueryError::new(
                        QueryErrorKind::GraphInvariant,
                        "query_graph_invariant",
                        format!("index references absent graph node {id}"),
                    )
                })?;
                nodes.push(node);
            }
            if !nodes.is_empty() {
                selected_query = query;
                break;
            }
        }
        let truncated = if nodes.len() == read_envelope {
            let last = nodes.last().ok_or_else(|| {
                QueryError::new(
                    QueryErrorKind::Internal,
                    "term_posting_invariant",
                    "nonzero discovery term envelope returned no rows",
                )
            })?;
            connection
                .query_row(
                    "SELECT EXISTS(
                       SELECT 1
                         FROM node_fts JOIN nodes n ON n.id = node_fts.node_id
                        WHERE node_fts MATCH ?1 AND n.id > ?2
                     )",
                    params![selected_query, last.id],
                    |row| row.get::<_, bool>(0),
                )
                .map_err(sql_error)?
        } else {
            false
        };
        let node_ids_decoded = u64::try_from(nodes.len()).unwrap_or(u64::MAX);
        nodes.truncate(candidate_limit);
        let matched = concepts.iter().cloned().collect::<BTreeSet<_>>();
        let matched_concepts = nodes
            .iter()
            .map(|node| (node.id.clone(), matched.clone()))
            .collect();
        Ok(TermCandidateRead {
            node_ids_decoded,
            nodes,
            matched_concepts,
            truncated,
            chunks_decoded: 0,
        })
    }

    pub(crate) fn discovery_relationship_sources(
        &self,
        backend: &PinnedDiscoveryBackend<'_>,
        concept: &str,
        candidate_limit: usize,
    ) -> Result<RelationshipCandidateRead, QueryError> {
        if let Some(read) = backend.store_relationship_sources(concept, candidate_limit)? {
            return Ok(read);
        }
        let connection = self.connection.as_ref().ok_or_else(|| {
            QueryError::new(
                QueryErrorKind::Internal,
                "query_index_missing",
                "materialized query engine has no search index",
            )
        })?;
        let read_envelope = (candidate_limit.max(GRAPH_TERM_POSTING_CHUNK_ITEMS)
            / GRAPH_TERM_POSTING_CHUNK_ITEMS)
            .saturating_mul(GRAPH_TERM_POSTING_CHUNK_ITEMS);
        let sql_limit = i64::try_from(read_envelope).unwrap_or(i64::MAX);
        let mut statement = connection
            .prepare(
                "SELECT source_id
                   FROM relationship_terms
                  WHERE term = ?1
                  ORDER BY source_id
                  LIMIT ?2",
            )
            .map_err(sql_error)?;
        let mut rows = statement
            .query(params![concept, sql_limit])
            .map_err(sql_error)?;
        let mut source_ids = Vec::new();
        while let Some(row) = rows.next().map_err(sql_error)? {
            source_ids.push(row.get(0).map_err(sql_error)?);
        }
        let truncated = if source_ids.len() == read_envelope {
            let last = source_ids.last().ok_or_else(|| {
                QueryError::new(
                    QueryErrorKind::Internal,
                    "relationship_posting_invariant",
                    "nonzero relationship posting envelope returned no rows",
                )
            })?;
            connection
                .query_row(
                    "SELECT EXISTS(
                       SELECT 1 FROM relationship_terms
                        WHERE term = ?1 AND source_id > ?2
                     )",
                    params![concept, last],
                    |row| row.get::<_, bool>(0),
                )
                .map_err(sql_error)?
        } else {
            false
        };
        let node_ids_decoded = u64::try_from(source_ids.len()).unwrap_or(u64::MAX);
        source_ids.truncate(candidate_limit);
        Ok(RelationshipCandidateRead {
            node_ids_decoded,
            source_ids,
            truncated,
            chunks_decoded: 0,
        })
    }

    pub(crate) fn discovery_relationship_source_matches_term(
        &self,
        backend: &PinnedDiscoveryBackend<'_>,
        source_id: &str,
        concept: &str,
    ) -> Result<bool, QueryError> {
        if let Some(matches) = backend.store_relationship_source_matches_term(source_id, concept)? {
            return Ok(matches);
        }
        let connection = self.connection.as_ref().ok_or_else(|| {
            QueryError::new(
                QueryErrorKind::Internal,
                "query_index_missing",
                "materialized query engine has no search index",
            )
        })?;
        connection
            .query_row(
                "SELECT 1
                   FROM relationship_terms
                  WHERE term = ?1 AND source_id = ?2",
                params![concept, source_id],
                |_| Ok(()),
            )
            .optional()
            .map(|found| found.is_some())
            .map_err(sql_error)
    }

    pub(crate) fn discovery_relationship_targets(
        &self,
        backend: &PinnedDiscoveryBackend<'_>,
        source_id: &str,
        concepts: &BTreeSet<String>,
        limit: usize,
    ) -> Result<RelationshipTargetRead, QueryError> {
        if concepts.is_empty() || limit == 0 {
            return Ok(RelationshipTargetRead {
                target_ids: Vec::new(),
                truncated: false,
                ids_decoded: 0,
            });
        }
        if let Some(read) = backend.store_relationship_targets(source_id, concepts, limit)? {
            return Ok(read);
        }
        let connection = self.connection.as_ref().ok_or_else(|| {
            QueryError::new(
                QueryErrorKind::Internal,
                "query_index_missing",
                "materialized query engine has no search index",
            )
        })?;
        let per_term_limit = limit.div_ceil(concepts.len());
        let mut target_ids = BTreeSet::new();
        let mut ids_decoded = 0_u64;
        let mut truncated = false;
        for concept in concepts {
            let decoded = usize::try_from(ids_decoded).unwrap_or(usize::MAX);
            let remaining = limit.saturating_sub(decoded);
            if remaining == 0 {
                truncated = true;
                break;
            }
            let row_limit = remaining.min(per_term_limit);
            let sql_limit = i64::try_from(row_limit).unwrap_or(i64::MAX);
            let mut statement = connection
                .prepare(
                    "SELECT target_id
                       FROM relationship_term_targets
                      WHERE source_id = ?1 AND term = ?2
                      ORDER BY target_id
                      LIMIT ?3",
                )
                .map_err(sql_error)?;
            let mut rows = statement
                .query(params![source_id, concept, sql_limit])
                .map_err(sql_error)?;
            let mut term_rows = 0_usize;
            while let Some(row) = rows.next().map_err(sql_error)? {
                term_rows = term_rows.saturating_add(1);
                target_ids.insert(row.get(0).map_err(sql_error)?);
            }
            ids_decoded = ids_decoded.saturating_add(u64::try_from(term_rows).unwrap_or(u64::MAX));
            truncated |= term_rows == row_limit;
        }
        Ok(RelationshipTargetRead {
            target_ids: target_ids.into_iter().collect(),
            truncated,
            ids_decoded,
        })
    }

    pub fn callers(&self, request: CallRequest) -> Result<CodeQueryResponse, QueryError> {
        self.call_neighbors_instrumented(request, true, &mut QueryInstrumentation::default())
    }

    pub fn callees(&self, request: CallRequest) -> Result<CodeQueryResponse, QueryError> {
        self.call_neighbors_instrumented(request, false, &mut QueryInstrumentation::default())
    }

    pub(crate) fn call_neighbors_instrumented(
        &self,
        request: CallRequest,
        inbound: bool,
        instrumentation: &mut QueryInstrumentation,
    ) -> Result<CodeQueryResponse, QueryError> {
        validate_limits(&request.limits)?;
        let operation = if inbound {
            CodeQueryOperation::Callers
        } else {
            CodeQueryOperation::Callees
        };
        let mut response = CodeQueryResponse::empty(operation, request.limits.clone());
        let recall_started = Instant::now();
        let role = if inbound {
            StructuralOperandRole::CallersTarget
        } else {
            StructuralOperandRole::CalleesSource
        };
        let seed = self.resolve_symbol(
            &request.symbol,
            &mut response,
            Some(role),
            request.include_heuristic,
            instrumentation,
        )?;
        instrumentation.recall += recall_started.elapsed();
        let Some(seed) = seed else {
            let execution_started = Instant::now();
            let response = self.finish_response(&mut response);
            instrumentation.execution += execution_started.elapsed();
            return response;
        };
        let execution_started = Instant::now();
        let kinds: &[EdgeKind] = if inbound {
            &[EdgeKind::Calls, EdgeKind::RoutesTo]
        } else {
            &[EdgeKind::Calls]
        };
        let max_edges = usize::try_from(request.limits.max_edges).unwrap_or(usize::MAX);
        let (selected_edges, truncated) = self.backend.matching_bounded(
            &seed,
            inbound,
            kinds,
            request.include_heuristic,
            max_edges,
        )?;
        instrumentation.work.nodes_expanded = instrumentation.work.nodes_expanded.saturating_add(1);
        instrumentation.work.edges_expanded = instrumentation
            .work
            .edges_expanded
            .saturating_add(u64::try_from(selected_edges.len()).unwrap_or(u64::MAX));
        if truncated {
            instrumentation.work.edges_expanded =
                instrumentation.work.edges_expanded.saturating_add(1);
        }
        response.truncated |= truncated;
        let mut ids = HashSet::from([seed.clone()]);
        for edge in &selected_edges {
            ids.insert(edge.source.clone());
            ids.insert(edge.target.clone());
            response.edges.push(query_edge(edge));
        }
        self.add_nodes(&ids, &mut response)?;
        let response = self.finish_response(&mut response);
        instrumentation.execution += execution_started.elapsed();
        response
    }

    pub fn impact(&self, request: ImpactRequest) -> Result<CodeQueryResponse, QueryError> {
        self.impact_instrumented(request, &mut QueryInstrumentation::default())
    }

    pub(crate) fn impact_instrumented(
        &self,
        request: ImpactRequest,
        instrumentation: &mut QueryInstrumentation,
    ) -> Result<CodeQueryResponse, QueryError> {
        validate_limits(&request.limits)?;
        let mut response =
            CodeQueryResponse::empty(CodeQueryOperation::Impact, request.limits.clone());
        let recall_started = Instant::now();
        let seed = self.resolve_symbol(
            &request.symbol,
            &mut response,
            Some(StructuralOperandRole::ImpactTarget),
            request.include_heuristic,
            instrumentation,
        )?;
        instrumentation.recall += recall_started.elapsed();
        let Some(seed) = seed else {
            let execution_started = Instant::now();
            let response = self.finish_response(&mut response);
            instrumentation.execution += execution_started.elapsed();
            return response;
        };
        let execution_started = Instant::now();
        let max_depth = usize::try_from(request.limits.max_depth).unwrap_or(usize::MAX);
        let max_nodes = usize::try_from(request.limits.max_nodes).unwrap_or(usize::MAX);
        let mut queue =
            VecDeque::from([(seed.clone(), Vec::<String>::new(), Vec::<String>::new())]);
        let mut visited = HashSet::from([seed.clone()]);
        let max_edges = usize::try_from(request.limits.max_edges).unwrap_or(usize::MAX);
        let mut selected_edges = HashSet::new();
        while let Some((node, path_nodes, path_edges)) = queue.pop_front() {
            instrumentation.work.nodes_expanded =
                instrumentation.work.nodes_expanded.saturating_add(1);
            if path_edges.len() >= max_depth {
                continue;
            }
            let remaining_edges = max_edges.saturating_sub(selected_edges.len());
            let (incoming, incoming_truncated) = self.backend.matching_bounded(
                &node,
                true,
                IMPACT_KINDS,
                request.include_heuristic,
                remaining_edges,
            )?;
            instrumentation.work.edges_expanded = instrumentation
                .work
                .edges_expanded
                .saturating_add(u64::try_from(incoming.len()).unwrap_or(u64::MAX));
            if incoming_truncated {
                instrumentation.work.edges_expanded =
                    instrumentation.work.edges_expanded.saturating_add(1);
            }
            response.truncated |= incoming_truncated;
            for edge in incoming {
                if selected_edges.len() >= max_edges {
                    response.truncated = true;
                    break;
                }
                if visited.insert(edge.source.clone()) {
                    if visited.len() > max_nodes {
                        visited.remove(&edge.source);
                        response.truncated = true;
                        break;
                    }
                    selected_edges.insert(edge.id.clone());
                    let mut nodes = path_nodes.clone();
                    if nodes.is_empty() {
                        nodes.push(seed.clone());
                    }
                    nodes.push(edge.source.clone());
                    let mut edges = path_edges.clone();
                    edges.push(edge.id.clone());
                    response.paths.push(self.path_record(&nodes, &edges)?);
                    queue.push_back((edge.source.clone(), nodes, edges));
                } else {
                    selected_edges.insert(edge.id.clone());
                }
            }
            if response.truncated {
                break;
            }
        }
        let ids = visited;
        self.add_nodes(&ids, &mut response)?;
        self.add_edges(&selected_edges, &mut response)?;
        self.apply_path_bound(&mut response);
        let response = self.finish_response(&mut response);
        instrumentation.execution += execution_started.elapsed();
        response
    }

    pub fn explore(&self, request: ExploreRequest) -> Result<CodeQueryResponse, QueryError> {
        self.explore_instrumented(request, true, &mut QueryInstrumentation::default())
    }

    /// Return the bounded structural subgraph without reading source files.
    ///
    /// This is the backend-neutral comparison route for graph engines. Public
    /// `explore` behavior remains unchanged and still includes verified source.
    pub fn structural_subgraph(
        &self,
        request: ExploreRequest,
    ) -> Result<CodeQueryResponse, QueryError> {
        self.explore_instrumented(request, false, &mut QueryInstrumentation::default())
    }

    fn explore_instrumented(
        &self,
        request: ExploreRequest,
        include_files: bool,
        instrumentation: &mut QueryInstrumentation,
    ) -> Result<CodeQueryResponse, QueryError> {
        validate_limits(&request.limits)?;
        if request.symbols.len()
            > usize::try_from(request.limits.max_candidates).unwrap_or(usize::MAX)
        {
            return Err(QueryError::new(
                QueryErrorKind::InvalidParameter,
                "too_many_explore_symbols",
                format!(
                    "explore requested {} symbols but maxCandidates is {}",
                    request.symbols.len(),
                    request.limits.max_candidates
                ),
            ));
        }
        let mut response =
            CodeQueryResponse::empty(CodeQueryOperation::Explore, request.limits.clone());
        let mut seeds = Vec::new();
        let recall_started = Instant::now();
        for symbol in &request.symbols {
            if let Some(seed) = self.resolve_symbol(
                symbol,
                &mut response,
                None,
                request.include_heuristic,
                instrumentation,
            )? {
                seeds.push(seed);
            }
        }
        instrumentation.recall += recall_started.elapsed();
        let execution_started = Instant::now();
        seeds.sort();
        seeds.dedup();
        let mut ids = seeds.iter().cloned().collect::<HashSet<_>>();
        let mut edge_ids = HashSet::new();
        let mut budget = TraversalBudget::new(&request.limits);
        for pair in seeds.windows(2) {
            if !budget.can_start_pair() {
                response.truncated = true;
                break;
            }
            if let [source, target] = pair
                && let (Some((nodes, edges)), truncated) = self.shortest_path(
                    source,
                    target,
                    request.include_heuristic,
                    &request.limits,
                    &mut budget,
                    false,
                )?
            {
                response.truncated |= truncated;
                ids.extend(nodes.iter().cloned());
                edge_ids.extend(edges.iter().cloned());
                response.paths.push(self.path_record(&nodes, &edges)?);
            }
        }
        self.add_nodes(&ids, &mut response)?;
        self.add_edges(&edge_ids, &mut response)?;
        if include_files {
            self.add_verified_files(&request.root, &mut response)?;
        }
        self.apply_path_bound(&mut response);
        budget.record_work(instrumentation);
        let response = self.finish_response(&mut response);
        instrumentation.execution += execution_started.elapsed();
        response
    }

    pub fn node_trail(&self, request: NodeTrailRequest) -> Result<CodeQueryResponse, QueryError> {
        self.node_trail_instrumented(request, &mut QueryInstrumentation::default())
    }

    pub(crate) fn node_trail_instrumented(
        &self,
        request: NodeTrailRequest,
        instrumentation: &mut QueryInstrumentation,
    ) -> Result<CodeQueryResponse, QueryError> {
        validate_limits(&request.limits)?;
        let mut response =
            CodeQueryResponse::empty(CodeQueryOperation::NodeTrail, request.limits.clone());
        let recall_started = Instant::now();
        let source = self.resolve_symbol(
            &request.source,
            &mut response,
            Some(StructuralOperandRole::TrailSource),
            request.include_heuristic,
            instrumentation,
        )?;
        let Some(source) = source else {
            instrumentation.recall += recall_started.elapsed();
            let execution_started = Instant::now();
            let response = self.finish_response(&mut response);
            instrumentation.execution += execution_started.elapsed();
            return response;
        };
        let target = self.resolve_symbol(
            &request.target,
            &mut response,
            Some(StructuralOperandRole::TrailTarget),
            request.include_heuristic,
            instrumentation,
        )?;
        instrumentation.recall += recall_started.elapsed();
        let Some(target) = target else {
            let execution_started = Instant::now();
            let response = self.finish_response(&mut response);
            instrumentation.execution += execution_started.elapsed();
            return response;
        };
        let execution_started = Instant::now();
        let mut budget = TraversalBudget::new(&request.limits);
        let (path, truncated) = self.shortest_path(
            &source,
            &target,
            request.include_heuristic,
            &request.limits,
            &mut budget,
            true,
        )?;
        response.truncated |= truncated;
        let Some((nodes, edges)) = path else {
            if truncated {
                budget.record_work(instrumentation);
                let response = self.finish_response(&mut response);
                instrumentation.execution += execution_started.elapsed();
                return response;
            }
            let (undirected_path, mismatch_truncated) = self.shortest_path(
                &source,
                &target,
                request.include_heuristic,
                &request.limits,
                &mut budget,
                false,
            )?;
            budget.record_work(instrumentation);
            response.truncated |= mismatch_truncated;
            if undirected_path.is_some() {
                response.diagnostics.push(QueryDiagnostic {
                    code: QueryDiagnosticCode::DirectionMismatch,
                    message: format!(
                        "A trail connects {source} and {target}, but not in the requested source-to-target direction"
                    ),
                    node_id: Some(source),
                    path: None,
                });
            } else if !mismatch_truncated {
                response.diagnostics.push(QueryDiagnostic {
                    code: QueryDiagnosticCode::NoMatch,
                    message: format!("No bounded directed trail connects {source} to {target}"),
                    node_id: Some(source),
                    path: None,
                });
            }
            let response = self.finish_response(&mut response);
            instrumentation.execution += execution_started.elapsed();
            return response;
        };
        let ids = nodes.iter().cloned().collect::<HashSet<_>>();
        self.add_nodes(&ids, &mut response)?;
        let edge_ids = edges.iter().cloned().collect::<HashSet<_>>();
        self.add_edges(&edge_ids, &mut response)?;
        response.paths.push(self.path_record(&nodes, &edges)?);
        budget.record_work(instrumentation);
        let response = self.finish_response(&mut response);
        instrumentation.execution += execution_started.elapsed();
        response
    }

    #[must_use]
    pub fn graph_path(&self) -> &std::path::Path {
        &self.graph_path
    }

    #[must_use]
    pub fn index_path(&self) -> &std::path::Path {
        &self.index_path
    }

    #[must_use]
    pub const fn engine_kind(&self) -> QueryEngineKind {
        self.engine_kind
    }

    /// Immutable identity supplied by the selected graph engine.
    #[must_use]
    pub fn graph_identity(&self) -> &str {
        &self.graph_identity
    }

    /// Build generation recorded by the selected graph snapshot.
    #[must_use]
    pub fn build_generation_identity(&self) -> &str {
        &self.build_generation_identity
    }

    fn resolve_symbol(
        &self,
        query: &str,
        response: &mut CodeQueryResponse,
        role: Option<StructuralOperandRole>,
        include_heuristic: bool,
        instrumentation: &mut QueryInstrumentation,
    ) -> Result<Option<String>, QueryError> {
        if let Some(node) = self.backend.node_by_id(query)? {
            instrumentation.work.candidates_read =
                instrumentation.work.candidates_read.saturating_add(1);
            return Ok(Some(node.id));
        }
        let normalized = normalize_symbol(query);
        let candidate_limit = usize::try_from(response.limits.max_candidates).unwrap_or(usize::MAX);
        let (exact_nodes, exact_truncated) = self
            .backend
            .nodes_by_normalized_name(&normalized, candidate_limit)?;
        instrumentation.work.candidates_read = instrumentation
            .work
            .candidates_read
            .saturating_add(u64::try_from(exact_nodes.len()).unwrap_or(u64::MAX));
        let exact = exact_nodes
            .into_iter()
            .map(|node| node.id)
            .collect::<Vec<_>>();
        response.truncated |= exact_truncated;
        match exact.as_slice() {
            [node] if !exact_truncated => return Ok(Some(node.clone())),
            [] => {}
            _ => {
                response.diagnostics.push(QueryDiagnostic {
                    code: QueryDiagnosticCode::AmbiguousMatch,
                    message: if exact_truncated {
                        format!(
                            "Symbol {query:?} exceeded the {}-candidate resolution bound",
                            response.limits.max_candidates
                        )
                    } else {
                        format!("Symbol {query:?} matched {} nodes", exact.len())
                    },
                    node_id: None,
                    path: None,
                });
                return Ok(None);
            }
        }

        let prepared = self.prepare_search_query(query)?;
        if prepared.fts_query.is_empty() {
            response.diagnostics.push(QueryDiagnostic {
                code: QueryDiagnosticCode::NoMatch,
                message: format!("No symbol matched {query:?}"),
                node_id: None,
                path: None,
            });
            return Ok(None);
        }
        let admit = |_: &NodeRecord| true;
        let mut check = || Ok(());
        let assembly = self.assemble_search_candidates(
            query,
            &prepared.terms,
            &prepared.fts_query,
            CandidateAssemblyPolicy {
                max_candidates: candidate_limit,
                source_lookup_limit: candidate_limit,
                max_candidate_reads: usize::try_from(
                    compass_model::query_contract::MAX_INDEXED_CANDIDATE_NODES_READ,
                )
                .unwrap_or(usize::MAX),
                max_candidate_probes: usize::try_from(
                    compass_model::query_contract::MAX_INDEXED_CANDIDATE_PROBES,
                )
                .unwrap_or(usize::MAX),
                bounded_posting_work: false,
                admit: &admit,
                check: &mut check,
            },
            role,
            include_heuristic,
        )?;
        instrumentation.work.candidates_read = instrumentation
            .work
            .candidates_read
            .saturating_add(assembly.candidate_nodes_read);
        instrumentation.work.postings_decoded = instrumentation
            .work
            .postings_decoded
            .saturating_add(assembly.postings_decoded);
        instrumentation.work.edges_expanded = instrumentation
            .work
            .edges_expanded
            .saturating_add(assembly.relation_edges_examined);
        response.truncated |= assembly.truncated;
        let candidates = assembly.pool.into_vec();
        if candidates.is_empty() {
            response.diagnostics.push(QueryDiagnostic {
                code: QueryDiagnosticCode::NoMatch,
                message: format!("No symbol matched {query:?}"),
                node_id: None,
                path: None,
            });
            return Ok(None);
        }
        if response.truncated {
            response.diagnostics.push(QueryDiagnostic {
                code: QueryDiagnosticCode::AmbiguousMatch,
                message: format!(
                    "Symbol {query:?} could not be resolved uniquely within {} candidates",
                    response.limits.max_candidates
                ),
                node_id: None,
                path: None,
            });
            return Ok(None);
        }
        let candidate_count = candidates.len();
        let ranked =
            rank_search_candidates(query, &prepared.ranking_terms, candidates, candidate_limit);
        if let [candidate] = ranked.as_slice() {
            return Ok(Some(candidate.node.id.clone()));
        }
        if let [candidate, runner_up, ..] = ranked.as_slice()
            && resolution_rank_is_strictly_better(candidate, runner_up)
        {
            return Ok(Some(candidate.node.id.clone()));
        }
        response.diagnostics.push(QueryDiagnostic {
            code: QueryDiagnosticCode::AmbiguousMatch,
            message: format!(
                "Symbol {query:?} recalled {} candidates; provide a qualified name or exact ID",
                candidate_count
            ),
            node_id: None,
            path: None,
        });
        Ok(None)
    }

    fn add_nodes(
        &self,
        ids: &HashSet<String>,
        response: &mut CodeQueryResponse,
    ) -> Result<(), QueryError> {
        let max = usize::try_from(response.limits.max_nodes).unwrap_or(usize::MAX);
        let mut nodes = Vec::with_capacity(ids.len().min(max));
        for id in ids {
            if let Some(node) = self.backend.node_by_id(id)? {
                nodes.push(node);
            }
        }
        nodes.sort_by(|left, right| left.id.cmp(&right.id));
        if nodes.len() > max {
            nodes.truncate(max);
            response.truncated = true;
        }
        response.nodes.extend(nodes.iter().map(query_node));
        Ok(())
    }

    fn add_edges(
        &self,
        ids: &HashSet<String>,
        response: &mut CodeQueryResponse,
    ) -> Result<(), QueryError> {
        let mut edges = Vec::with_capacity(ids.len());
        for id in ids {
            if let Some(edge) = self.backend.edge_by_id(id)? {
                edges.push(edge);
            }
        }
        edges.sort_by(|left, right| left.id.cmp(&right.id));
        response.edges.extend(edges.iter().map(query_edge));
        Ok(())
    }

    fn path_record(&self, nodes: &[String], edges: &[String]) -> Result<QueryPath, QueryError> {
        let mut selected = Vec::with_capacity(edges.len());
        for edge in edges {
            if let Some(record) = self.backend.edge_by_id(edge)? {
                selected.push(record);
            }
        }
        Ok(path_record(nodes, edges, &selected))
    }

    fn shortest_path(
        &self,
        source: &str,
        target: &str,
        include_heuristic: bool,
        limits: &compass_model::query_contract::CodeQueryLimits,
        budget: &mut TraversalBudget,
        directed: bool,
    ) -> Result<BoundedPathResult, QueryError> {
        let max_depth = usize::try_from(limits.max_depth).unwrap_or(usize::MAX);
        if !budget.consume_node() {
            return Ok((None, true));
        }
        let mut queue = VecDeque::from([(source.to_owned(), 0_usize)]);
        let mut visited = HashSet::from([source.to_owned()]);
        let mut predecessor = HashMap::<String, (String, String)>::new();
        let mut truncated = false;
        while let Some((node, depth)) = queue.pop_front() {
            if node == target {
                let mut nodes = vec![target.to_owned()];
                let mut edges = Vec::new();
                let mut cursor = target;
                while cursor != source {
                    let Some((previous, edge)) = predecessor.get(cursor) else {
                        return Ok((None, truncated));
                    };
                    edges.push(edge.clone());
                    nodes.push(previous.clone());
                    cursor = previous;
                }
                nodes.reverse();
                edges.reverse();
                return Ok((Some((nodes, edges)), truncated));
            }
            if depth >= max_depth {
                continue;
            }
            let mut adjacent = Vec::new();
            let (incident, incident_truncated) = if directed {
                self.backend.matching_bounded(
                    &node,
                    false,
                    ALL_EDGE_KINDS,
                    include_heuristic,
                    budget.remaining_edges,
                )?
            } else {
                self.backend
                    .incident_bounded(&node, include_heuristic, budget.remaining_edges)?
            };
            truncated |= incident_truncated;
            for edge in incident {
                if !budget.consume_edge() {
                    truncated = true;
                    break;
                }
                if edge.source == node {
                    adjacent.push((edge.target.clone(), edge));
                } else if !directed && edge.target == node {
                    adjacent.push((edge.source.clone(), edge));
                }
            }
            adjacent.sort_by(|left, right| {
                evidence_quality(&right.1)
                    .cmp(&evidence_quality(&left.1))
                    .then_with(|| left.1.id.cmp(&right.1.id))
            });
            for (next, edge) in adjacent {
                if visited.contains(&next) {
                    continue;
                }
                if !budget.consume_node() {
                    truncated = true;
                    continue;
                }
                visited.insert(next.clone());
                predecessor.insert(next.clone(), (node.clone(), edge.id.clone()));
                queue.push_back((next, depth + 1));
            }
        }
        Ok((None, truncated))
    }

    fn add_verified_files(
        &self,
        requested_root: &str,
        response: &mut CodeQueryResponse,
    ) -> Result<(), QueryError> {
        let root = if requested_root.is_empty() {
            default_repository_root(&self.graph_path)
        } else {
            PathBuf::from(requested_root)
        };
        let files = response
            .nodes
            .iter()
            .filter_map(|node| node.source.as_ref().map(|source| source.file.clone()))
            .collect::<HashSet<_>>();
        let max_total = response.limits.max_source_bytes;
        let per_file = max_total / u64::try_from(files.len().max(1)).unwrap_or(1);
        for path in files {
            let Some(record) = self.backend.file_by_path(&path)? else {
                continue;
            };
            match verified_source(&root, &path, &record.content_digest, per_file)? {
                VerifiedSource::Fresh { source, truncated } => {
                    if truncated {
                        response.truncated = true;
                    }
                    response.files.push(QueryFile {
                        path,
                        content_digest: record.content_digest.clone(),
                        source: Some(source),
                        truncated,
                    });
                }
                VerifiedSource::Stale { actual } => {
                    response.files.push(QueryFile {
                        path: path.clone(),
                        content_digest: record.content_digest.clone(),
                        source: None,
                        truncated: false,
                    });
                    response.diagnostics.push(QueryDiagnostic {
                        code: QueryDiagnosticCode::StaleSourceDigest,
                        message: format!(
                            "Indexed digest {} differs from current digest {actual}",
                            record.content_digest
                        ),
                        node_id: None,
                        path: Some(path),
                    });
                }
            }
        }
        Ok(())
    }

    fn apply_path_bound(&self, response: &mut CodeQueryResponse) {
        let max = usize::try_from(response.limits.max_paths).unwrap_or(usize::MAX);
        if response.paths.len() > max {
            response.paths.truncate(max);
            response.truncated = true;
        }
    }

    fn finish_response(
        &self,
        response: &mut CodeQueryResponse,
    ) -> Result<CodeQueryResponse, QueryError> {
        if self.program.is_some() {
            join_program_evidence(response, self.program.as_ref());
        } else {
            response.sort_stable();
        }
        self.enforce_graph_bounds(response);
        if response.truncated {
            response.diagnostics.push(QueryDiagnostic {
                code: QueryDiagnosticCode::BoundedTruncation,
                message: "One or more query bounds truncated the response".to_owned(),
                node_id: None,
                path: None,
            });
        }
        if let Some(message) = &self.partial_graph_message {
            response.diagnostics.push(QueryDiagnostic {
                code: QueryDiagnosticCode::IncompleteCoverage,
                message: message.clone(),
                node_id: None,
                path: None,
            });
        }
        response.sort_stable();
        enforce_response_size(response)?;
        Ok(response.clone())
    }

    fn enforce_graph_bounds(&self, response: &mut CodeQueryResponse) {
        response.sort_stable();
        let max_nodes = usize::try_from(response.limits.max_nodes).unwrap_or(usize::MAX);
        if response.nodes.len() > max_nodes {
            response.nodes.truncate(max_nodes);
            response.truncated = true;
        }
        let max_edges = usize::try_from(response.limits.max_edges).unwrap_or(usize::MAX);
        if response.edges.len() > max_edges {
            response.edges.truncate(max_edges);
            response.truncated = true;
        }
        let node_ids = response
            .nodes
            .iter()
            .map(|node| node.id.as_str())
            .collect::<HashSet<_>>();
        let edge_count = response.edges.len();
        response.edges.retain(|edge| {
            node_ids.contains(edge.source.as_str()) && node_ids.contains(edge.target.as_str())
        });
        response.truncated |= response.edges.len() != edge_count;
        let edge_ids = response
            .edges
            .iter()
            .map(|edge| edge.id.as_str())
            .collect::<HashSet<_>>();
        let path_count = response.paths.len();
        response.paths.retain(|path| {
            path.node_ids
                .iter()
                .all(|node| node_ids.contains(node.as_str()))
                && path
                    .edge_ids
                    .iter()
                    .all(|edge| edge_ids.contains(edge.as_str()))
        });
        response.truncated |= response.paths.len() != path_count;
    }
}

fn default_repository_root(graph_path: &Path) -> PathBuf {
    let Some(graph_directory) = graph_path.parent() else {
        return PathBuf::from(".");
    };
    if graph_directory
        .parent()
        .and_then(Path::file_name)
        .is_some_and(|name| name == "snapshots")
    {
        return graph_directory
            .ancestors()
            .nth(3)
            .unwrap_or_else(|| Path::new("."))
            .to_path_buf();
    }
    graph_directory
        .parent()
        .unwrap_or_else(|| Path::new("."))
        .to_path_buf()
}

fn path_record(nodes: &[String], edges: &[String], selected: &[EdgeRecord]) -> QueryPath {
    query_path_from_records(nodes, edges, selected)
}

pub(crate) fn normalize_symbol(value: &str) -> String {
    normalize_query_symbol(value)
}

fn is_operation_role_declaration(node: &NodeRecord) -> bool {
    matches!(
        node.kind,
        NodeKind::Class
            | NodeKind::Struct
            | NodeKind::Interface
            | NodeKind::Trait
            | NodeKind::Protocol
            | NodeKind::Enum
            | NodeKind::TypeAlias
    ) && node.source_file().is_some_and(|file| !file.is_empty())
        && compass_model::search::identifier_search_terms(&node.name)
            .iter()
            .any(|term| compass_model::search::OPERATION_ROLE_TOKENS.contains(&term.as_str()))
}

fn add_admitted_candidates(
    pool: &mut SearchCandidatePool,
    source: CandidateSource,
    nodes: Vec<NodeRecord>,
    admit: &dyn Fn(&NodeRecord) -> bool,
) {
    for node in nodes {
        if admit(&node) {
            let _ = pool.add(source, node);
        }
    }
}

fn is_heuristic(edge: &EdgeRecord) -> bool {
    edge.evidence
        .iter()
        .any(|evidence| evidence.origin == compass_model::provenance::EvidenceOrigin::Heuristic)
}

fn evidence_quality(edge: &EdgeRecord) -> u8 {
    edge.evidence
        .iter()
        .map(|evidence| match evidence.confidence {
            EvidenceConfidence::Exact => 3,
            EvidenceConfidence::Inferred => 2,
            EvidenceConfidence::Ambiguous => 1,
        })
        .max()
        .unwrap_or(0)
}

pub(crate) fn query_node(node: &NodeRecord) -> QueryNode {
    query_node_from_record(node)
}

pub(crate) fn query_edge(edge: &EdgeRecord) -> QueryEdge {
    query_edge_from_record(edge)
}

pub(crate) fn validate_limits(
    limits: &compass_model::query_contract::CodeQueryLimits,
) -> Result<(), QueryError> {
    if !limits.is_valid() {
        return Err(QueryError::new(
            QueryErrorKind::InvalidParameter,
            "invalid_code_query_limits",
            "every code query limit must be greater than zero",
        ));
    }
    if limits.max_candidates > MAX_CODE_QUERY_CANDIDATES {
        return Err(QueryError::new(
            QueryErrorKind::InvalidParameter,
            "code_query_limit_exceeded",
            format!(
                "maxCandidates {} exceeds the hard cap {MAX_CODE_QUERY_CANDIDATES}",
                limits.max_candidates
            ),
        ));
    }
    Ok(())
}

fn fts_query_from_terms(terms: &[String]) -> String {
    terms
        .iter()
        .map(|term| format!("\"{}\"*", term.replace('"', "\"\"")))
        .collect::<Vec<_>>()
        .join(" AND ")
}

fn validate_search_query_size(value: &str) -> Result<(), QueryError> {
    if value.len() > compass_model::query_contract::MAX_INDEXED_QUERY_BYTES {
        return Err(QueryError::new(
            QueryErrorKind::InvalidParameter,
            "search_query_too_large",
            format!(
                "search query exceeds {} bytes",
                compass_model::query_contract::MAX_INDEXED_QUERY_BYTES
            ),
        ));
    }
    Ok(())
}

pub(crate) fn search_query_terms(value: &str) -> Result<Vec<String>, QueryError> {
    validate_search_query_size(value)?;
    let terms = value
        .split(|character: char| !(character.is_alphanumeric() || character == '_'))
        .filter(|term| {
            !term.is_empty()
                && !matches!(
                    term.to_ascii_uppercase().as_str(),
                    "AND" | "OR" | "NOT" | "NEAR"
                )
        })
        .map(str::to_lowercase)
        .take(compass_model::query_contract::MAX_INDEXED_QUERY_TERMS.saturating_add(1))
        .collect::<Vec<_>>();
    validate_search_term_count(&terms)?;
    Ok(terms)
}

fn validate_search_term_count(terms: &[String]) -> Result<(), QueryError> {
    if terms.len() > compass_model::query_contract::MAX_INDEXED_QUERY_TERMS {
        return Err(QueryError::new(
            QueryErrorKind::InvalidParameter,
            "too_many_search_terms",
            format!(
                "search query exceeds {} terms",
                compass_model::query_contract::MAX_INDEXED_QUERY_TERMS
            ),
        ));
    }
    Ok(())
}

pub(crate) fn recall_fuzzy_term_variants(terms: &[String]) -> Vec<String> {
    let mut seen = terms.iter().cloned().collect::<BTreeSet<_>>();
    let mut variants = Vec::new();
    let eligible_terms = terms
        .iter()
        .filter(|term| term.chars().count() >= 4)
        .count();
    let per_term_limit = MAX_RECALL_FUZZY_VARIANTS_PER_TERM.min(
        MAX_RECALL_FUZZY_VARIANTS_TOTAL
            .checked_div(eligible_terms.max(1))
            .unwrap_or(1)
            .max(1),
    );
    let max_total_variants = eligible_terms.saturating_mul(per_term_limit).max(1);

    for term in terms {
        let chars = term.chars().collect::<Vec<_>>();
        if chars.len() < 4 {
            continue;
        }
        let remaining_capacity = max_total_variants.saturating_sub(variants.len());
        if remaining_capacity == 0 {
            break;
        }
        let per_term_target = remaining_capacity.min(per_term_limit);
        let mut emitted = 0_usize;
        let stripped = strip_diacritics(term);
        if stripped != *term && stripped.len() >= 3 && seen.insert(stripped.clone()) {
            variants.push(stripped);
            emitted = emitted.saturating_add(1);
        }

        // A transposition is a common typo and can recover the exact indexed
        // symbol without broadening the prefix scan.
        for index in 0..chars.len().saturating_sub(1) {
            if emitted >= per_term_target {
                break;
            }
            let mut variant = chars.clone();
            variant.swap(index, index + 1);
            if variant == chars {
                continue;
            }
            let variant = variant.iter().collect::<String>();
            if seen.insert(variant.clone()) {
                variants.push(variant);
                emitted = emitted.saturating_add(1);
            }
        }

        if emitted >= per_term_target {
            continue;
        }
        for index in 0..chars.len() {
            if emitted >= per_term_target {
                break;
            }
            let mut variant = chars.clone();
            variant.remove(index);
            if variant.len() < 3 {
                continue;
            }
            let variant = variant.iter().collect::<String>();
            if seen.insert(variant.clone()) {
                variants.push(variant);
                emitted = emitted.saturating_add(1);
            }
        }

        // Missing a repeated character is common (`calee` -> `callee`) and
        // can be recovered before the broader ASCII edit matrix.
        for index in 0..chars.len() {
            if emitted >= per_term_target {
                break;
            }
            let mut variant = chars.clone();
            variant.insert(index, chars[index]);
            let variant = variant.iter().collect::<String>();
            if seen.insert(variant.clone()) {
                variants.push(variant);
                emitted = emitted.saturating_add(1);
            }
        }

        if emitted >= per_term_target || !term.is_ascii() {
            continue;
        }
        // Bounded ASCII insertion and substitution recover the other common
        // single-edit typo classes while retaining exact index probes. The
        // replacement character is the outer loop so likely alphabetic edits
        // are reached before the fixed per-term ceiling.
        for replacement in "abcdefghijklmnopqrstuvwxyz0123456789_".chars() {
            for index in 0..=chars.len() {
                if emitted >= per_term_target {
                    break;
                }
                if index < chars.len() && chars[index] != replacement {
                    let mut variant = chars.clone();
                    variant[index] = replacement;
                    let variant = variant.iter().collect::<String>();
                    if seen.insert(variant.clone()) {
                        variants.push(variant);
                        emitted = emitted.saturating_add(1);
                    }
                }
                if emitted >= per_term_target {
                    break;
                }
                let mut variant = chars.clone();
                variant.insert(index, replacement);
                let variant = variant.iter().collect::<String>();
                if seen.insert(variant.clone()) {
                    variants.push(variant);
                    emitted = emitted.saturating_add(1);
                }
            }
            if emitted >= per_term_target {
                break;
            }
        }
    }

    variants
}

pub(crate) fn enforce_response_size(response: &mut CodeQueryResponse) -> Result<(), QueryError> {
    let bytes = serde_json::to_vec(response).map_err(|error| {
        QueryError::new(
            QueryErrorKind::Internal,
            "query_response_serialization",
            error.to_string(),
        )
    })?;
    if bytes.len() as u64 > response.limits.max_response_bytes {
        return Err(QueryError::new(
            QueryErrorKind::MemoryLimit,
            "query_response_too_large",
            format!(
                "query response is {} bytes; limit is {}",
                bytes.len(),
                response.limits.max_response_bytes
            ),
        ));
    }
    Ok(())
}

fn sql_error(error: rusqlite::Error) -> QueryError {
    QueryError::new(
        QueryErrorKind::CorruptArtifact,
        "query_index_error",
        error.to_string(),
    )
}

#[cfg(test)]
mod adjacency_tests {
    use compass_model::code_graph::{
        BuildMetadata, EdgeRecord, ExtractionStatus, FileRecord, GraphDocument, NodeKind,
        NodeRecord,
    };
    use compass_model::identity::{edge_id, file_id};
    use compass_model::provenance::{EvidenceConfidence, EvidenceOrigin, Provenance, SourceAnchor};
    use compass_model::validate_code_graph;

    use compass_model::query_contract::DiscoveryScopeKind;

    use super::{CodeAdjacencyIndex, EdgeKind, scope_kind_from_posting};

    #[test]
    fn discovery_scope_posting_mapping_is_closed() {
        for (posting, expected) in [
            ("community-id", DiscoveryScopeKind::Community),
            ("community-label", DiscoveryScopeKind::Community),
            ("source", DiscoveryScopeKind::Source),
            ("package", DiscoveryScopeKind::Package),
            ("node-id", DiscoveryScopeKind::Node),
            ("node-qname", DiscoveryScopeKind::Node),
        ] {
            assert_eq!(scope_kind_from_posting(posting), Some(expected));
        }
        assert_eq!(scope_kind_from_posting("future-extension"), None);
    }

    fn edge(id: &str, source: &str, kind: EdgeKind, target: &str) -> EdgeRecord {
        EdgeRecord {
            id: id.to_owned(),
            key: id.to_owned(),
            source: source.to_owned(),
            target: target.to_owned(),
            kind,
            occurrence_rule: None,
            relationship_site: None,
            details: None,
            evidence: Vec::new(),
            weight: None,
            context: None,
            deferred: false,
            diagnostics: Vec::new(),
        }
    }

    fn scale_anchor(start_byte: u64) -> SourceAnchor {
        let line = u32::try_from(start_byte.saturating_add(1)).unwrap_or(u32::MAX);
        SourceAnchor {
            file: "scale.rs".to_owned(),
            start_byte,
            end_byte: start_byte.saturating_add(1),
            start_line: line,
            start_column: 0,
            end_line: line,
            end_column: 1,
        }
    }

    fn scale_evidence(anchor: SourceAnchor) -> Provenance {
        Provenance {
            origin: EvidenceOrigin::Ast,
            extractor: "compass-query-scale".to_owned(),
            confidence: EvidenceConfidence::Exact,
            rule: None,
            anchors: vec![anchor],
            wiring_site: None,
            score: None,
            candidates: Vec::new(),
        }
    }

    #[test]
    fn directional_kind_indexes_match_full_edge_scans() {
        let mut graph = GraphDocument::empty_v1(BuildMetadata {
            builder_version: "test".to_owned(),
            schema_fingerprint: "schema".to_owned(),
            source_tree_digest: "tree".to_owned(),
            configuration_digest: "config".to_owned(),
            generation_id: "generation".to_owned(),
            source_commit: None,
        });
        graph.links = vec![
            edge("e:4", "a", EdgeKind::Calls, "b"),
            edge("e:2", "b", EdgeKind::RoutesTo, "a"),
            edge("e:3", "c", EdgeKind::Imports, "b"),
            edge("e:1", "b", EdgeKind::Calls, "b"),
        ];
        let adjacency = CodeAdjacencyIndex::build(&graph);
        let kinds = [EdgeKind::Calls, EdgeKind::RoutesTo, EdgeKind::Imports];

        for node in ["a", "b", "c", "absent"] {
            let mut expected_incident = graph
                .links
                .iter()
                .enumerate()
                .filter(|(_, edge)| edge.source == node || edge.target == node)
                .map(|(index, _)| index)
                .collect::<Vec<_>>();
            expected_incident
                .sort_by(|left, right| graph.links[*left].id.cmp(&graph.links[*right].id));
            assert_eq!(adjacency.incident(node, true), expected_incident);

            for inbound in [false, true] {
                for kind in kinds {
                    let mut expected = graph
                        .links
                        .iter()
                        .enumerate()
                        .filter(|(_, edge)| {
                            edge.kind == kind
                                && if inbound {
                                    edge.target == node
                                } else {
                                    edge.source == node
                                }
                        })
                        .map(|(index, _)| index)
                        .collect::<Vec<_>>();
                    expected
                        .sort_by(|left, right| graph.links[*left].id.cmp(&graph.links[*right].id));
                    assert_eq!(adjacency.matching(&graph, node, inbound, &[kind]), expected);
                }
            }
        }
        for (index, edge) in graph.links.iter().enumerate() {
            assert_eq!(adjacency.by_id(&edge.id), Some(index));
        }
    }

    #[test]
    fn trusted_adjacency_does_not_spend_budget_on_heuristic_edges() {
        let mut graph = GraphDocument::empty_v1(BuildMetadata {
            builder_version: "test".to_owned(),
            schema_fingerprint: "schema".to_owned(),
            source_tree_digest: "tree".to_owned(),
            configuration_digest: "config".to_owned(),
            generation_id: "generation".to_owned(),
            source_commit: None,
        });
        let mut heuristic = edge("e:1", "source", EdgeKind::Calls, "heuristic");
        heuristic.evidence.push(Provenance {
            origin: EvidenceOrigin::Heuristic,
            extractor: "test".to_owned(),
            confidence: EvidenceConfidence::Inferred,
            rule: Some("test-heuristic".to_owned()),
            anchors: Vec::new(),
            wiring_site: None,
            score: None,
            candidates: Vec::new(),
        });
        graph.links = vec![heuristic, edge("e:2", "source", EdgeKind::Calls, "trusted")];

        let adjacency = CodeAdjacencyIndex::build(&graph);
        assert_eq!(adjacency.incident("source", false), [1]);
        let (matching, truncated, examined) =
            adjacency.matching_bounded(&graph, "source", false, &[EdgeKind::Calls], false, 1);
        assert_eq!(matching, [1]);
        assert!(!truncated);
        assert_eq!(examined, 2);
    }

    #[test]
    fn bounded_matching_scales_with_response_budget_on_500k_edges()
    -> Result<(), Box<dyn std::error::Error>> {
        const NODES: usize = 100_000;
        const EDGES: usize = 500_000;
        const LIMIT: usize = 4;

        let mut graph = GraphDocument::empty_v1(BuildMetadata {
            builder_version: "test".to_owned(),
            schema_fingerprint: "schema".to_owned(),
            source_tree_digest: "tree".to_owned(),
            configuration_digest: "config".to_owned(),
            generation_id: "generation".to_owned(),
            source_commit: None,
        });
        graph.graph.files.push(FileRecord {
            id: file_id("scale.rs"),
            path: "scale.rs".to_owned(),
            language: Some("rust".to_owned()),
            content_digest: "sha256:scale-fixture".to_owned(),
            byte_size: u64::try_from(EDGES).unwrap_or(u64::MAX),
            generated: true,
            extraction_status: ExtractionStatus::Generated,
            extractor_versions: vec!["compass-query-scale/1".to_owned()],
            coverage: Vec::new(),
            diagnostics: Vec::new(),
        });
        graph.nodes.reserve(NODES);
        for index in 0..NODES {
            graph.nodes.push(NodeRecord {
                id: format!("n:{index:05}"),
                kind: NodeKind::Function,
                roles: Vec::new(),
                name: format!("f{index:05}"),
                qualified_name: format!("scale.f{index:05}"),
                language: None,
                framework: None,
                source: None,
                details: None,
                evidence: vec![scale_evidence(scale_anchor(0))],
                coverage: Vec::new(),
                diagnostics: Vec::new(),
                community: None,
            });
        }
        graph.links.reserve(EDGES);
        for index in 0..EDGES {
            let source = "n:00000";
            let target = format!("n:{:05}", index % NODES);
            let kind = if index % 2 == 0 {
                EdgeKind::Calls
            } else {
                EdgeKind::Overrides
            };
            let relationship_site = scale_anchor(u64::try_from(index).unwrap_or(u64::MAX));
            let id = edge_id(source, kind, &target, Some(&relationship_site), None);
            graph.links.push(EdgeRecord {
                id: id.clone(),
                key: id,
                source: source.to_owned(),
                target,
                kind,
                occurrence_rule: None,
                relationship_site: Some(relationship_site.clone()),
                details: None,
                evidence: vec![scale_evidence(relationship_site)],
                weight: None,
                context: None,
                deferred: false,
                diagnostics: Vec::new(),
            });
        }

        validate_code_graph(&graph)?;
        let adjacency = CodeAdjacencyIndex::build(&graph);
        let (matching, truncated, examined) = adjacency.matching_bounded(
            &graph,
            "n:00000",
            false,
            &[EdgeKind::Calls, EdgeKind::Overrides],
            true,
            LIMIT,
        );
        assert_eq!(matching.len(), LIMIT);
        assert!(truncated);
        assert!(
            examined <= (LIMIT + 1) * 2,
            "bounded lookup examined {examined} bucket heads"
        );
        println!(
            "{{\"nodes\":{},\"edges\":{EDGES},\"retained\":{},\"examined\":{examined}}}",
            graph.nodes.len(),
            matching.len()
        );
        Ok(())
    }
}

#[cfg(test)]
mod fuzzy_term_variant_tests {
    use super::{
        FuzzyLookupCache, MAX_RECALL_FUZZY_VARIANTS_PER_TERM, MAX_RECALL_FUZZY_VARIANTS_TOTAL,
        PreparedSearchQuery, SearchQueryCache, recall_fuzzy_term_variants,
    };

    #[test]
    fn recall_fuzzy_term_variants_is_deterministic_and_bounded() {
        let terms = vec![
            "fetchUsers".to_owned(),
            "dependencies".to_owned(),
            "résumé".to_owned(),
        ];
        let first = recall_fuzzy_term_variants(&terms);
        let second = recall_fuzzy_term_variants(&terms);
        assert_eq!(first, second, "variant generation must be deterministic");
        assert!(!first.is_empty());
        assert!(first.len() <= terms.len() * MAX_RECALL_FUZZY_VARIANTS_PER_TERM);
        assert!(first.len() <= MAX_RECALL_FUZZY_VARIANTS_TOTAL);
        assert!(first.iter().all(|variant| variant.len() >= 3));
        let unique = first.iter().collect::<std::collections::BTreeSet<_>>();
        assert_eq!(unique.len(), first.len());
        assert!(first.iter().any(|variant| variant == "resume"));
    }

    #[test]
    fn recall_fuzzy_term_variants_ignores_short_tokens() {
        let terms = vec![
            "a".to_owned(),
            "ab".to_owned(),
            "abc".to_owned(),
            "abcd".to_owned(),
        ];
        let variants = recall_fuzzy_term_variants(&terms);
        assert!(
            !variants
                .iter()
                .any(|variant| variant == "a" || variant == "ab" || variant == "abc")
        );
    }

    #[test]
    fn recall_fuzzy_term_variants_prioritizes_transpositions() {
        let variants = recall_fuzzy_term_variants(&["lits".to_owned()]);
        assert_eq!(variants.first().map(String::as_str), Some("ilts"));
        assert!(variants.iter().any(|variant| variant == "list"));
    }

    #[test]
    fn recall_fuzzy_term_variants_cover_single_edit_typo_classes() {
        for (typo, expected) in [
            ("cace_key", "cache_key"),
            ("callar", "caller"),
            ("calee", "callee"),
        ] {
            let variants = recall_fuzzy_term_variants(&[typo.to_owned()]);
            assert!(variants.iter().any(|variant| variant == expected));
            assert!(variants.len() <= MAX_RECALL_FUZZY_VARIANTS_TOTAL);
        }
    }

    fn prepared(term: &str) -> PreparedSearchQuery {
        PreparedSearchQuery {
            terms: vec![term.to_owned()],
            ranking_terms: vec![term.to_owned()],
            fts_query: format!("\"{term}\"*"),
        }
    }

    #[test]
    fn search_query_cache_is_bounded_and_refreshes_recent_entries() {
        let mut cache = SearchQueryCache {
            entries: Default::default(),
            order: Default::default(),
            capacity: 2,
        };
        cache.insert("one".to_owned(), prepared("one"));
        cache.insert("two".to_owned(), prepared("two"));
        assert!(cache.get("one").is_some());
        cache.insert("three".to_owned(), prepared("three"));

        assert!(cache.get("one").is_some());
        assert!(cache.get("two").is_none());
        assert!(cache.get("three").is_some());
        assert_eq!(cache.entries.len(), 2);
    }

    #[test]
    fn fuzzy_lookup_cache_is_bounded_and_keys_limits() {
        let mut cache = FuzzyLookupCache {
            entries: Default::default(),
            order: Default::default(),
            capacity: 2,
        };
        cache.insert("one".to_owned(), 1, (Vec::new(), false));
        cache.insert("one".to_owned(), 2, (Vec::new(), true));
        assert_eq!(cache.get("one", 1), Some((Vec::new(), false)));
        cache.insert("three".to_owned(), 1, (Vec::new(), false));
        assert_eq!(cache.get("one", 2), None);
        assert!(cache.get("one", 1).is_some());
        assert!(cache.get("three", 1).is_some());
    }
}
