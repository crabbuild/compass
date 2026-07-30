use std::collections::{HashMap, HashSet, VecDeque};
use std::path::{Path, PathBuf};

use compass_ir::ProgramBundle;
use compass_model::code_graph::{EdgeKind, EdgeRecord, GraphDocument, NodeRecord};
use compass_model::provenance::{EvidenceConfidence, ResolutionState};
use compass_model::query_contract::{
    CallRequest, CodeQueryOperation, CodeQueryResponse, ExploreRequest, ImpactRequest,
    NodeTrailRequest, QueryDiagnostic, QueryDiagnosticCode, QueryEdge, QueryEvidence,
    QueryEvidenceLayer, QueryFile, QueryNode, QueryPath, SearchHit, SearchRequest,
};
use rusqlite::{Connection, params};

use crate::cql::{QueryError, QueryErrorKind};
use crate::join_program_evidence;
use crate::source::{VerifiedSource, verified_source};

type GraphPath = (Vec<String>, Vec<String>);
type BoundedPathResult = (Option<GraphPath>, bool);
const MAX_CODE_QUERY_CANDIDATES: u32 = 256;

struct TraversalBudget {
    remaining_nodes: usize,
    remaining_edges: usize,
}

impl TraversalBudget {
    fn new(limits: &compass_model::query_contract::CodeQueryLimits) -> Self {
        Self {
            remaining_nodes: usize::try_from(limits.max_nodes).unwrap_or(usize::MAX),
            remaining_edges: usize::try_from(limits.max_edges).unwrap_or(usize::MAX),
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
            true
        }
    }

    fn consume_edge(&mut self) -> bool {
        if self.remaining_edges == 0 {
            false
        } else {
            self.remaining_edges -= 1;
            true
        }
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
];

pub struct CodeQueryEngine {
    pub(crate) graph: GraphDocument,
    pub(crate) program: Option<ProgramBundle>,
    pub(crate) connection: Connection,
    pub(crate) graph_path: PathBuf,
    pub(crate) index_path: PathBuf,
    pub(crate) adjacency: CodeAdjacencyIndex,
    pub(crate) lookup: CodeLookupIndex,
}

pub(crate) struct CodeLookupIndex {
    node_by_id: HashMap<String, usize>,
    nodes_by_normalized_name: HashMap<String, Vec<usize>>,
    file_by_path: HashMap<String, usize>,
}

impl CodeLookupIndex {
    pub(crate) fn build(graph: &GraphDocument) -> Self {
        let mut lookup = Self {
            node_by_id: HashMap::with_capacity(graph.nodes.len()),
            nodes_by_normalized_name: HashMap::new(),
            file_by_path: HashMap::with_capacity(graph.graph.files.len()),
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
        }
        for nodes in lookup.nodes_by_normalized_name.values_mut() {
            nodes.sort_by(|left, right| graph.nodes[*left].id.cmp(&graph.nodes[*right].id));
            nodes.dedup();
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

    fn file_by_path(&self, path: &str) -> Option<usize> {
        self.file_by_path.get(path).copied()
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
        validate_limits(&request.limits)?;
        let query = fts_query(&request.query)?;
        let mut response =
            CodeQueryResponse::empty(CodeQueryOperation::Search, request.limits.clone());
        if query.is_empty() {
            response.diagnostics.push(QueryDiagnostic {
                code: QueryDiagnosticCode::NoMatch,
                message: "Search query contains no searchable terms".to_owned(),
                node_id: None,
                path: None,
            });
            return Ok(response);
        }
        let mut statement = self
            .connection
            .prepare(
                "SELECT n.id, n.name, n.qualified_name, bm25(node_fts)
                 FROM node_fts JOIN nodes n ON n.id = node_fts.node_id
                 WHERE node_fts MATCH ?1
                 LIMIT ?2",
            )
            .map_err(sql_error)?;
        let candidate_limit =
            i64::from(request.limits.max_candidates.max(request.limits.max_nodes));
        let mut rows = statement
            .query(params![query, candidate_limit])
            .map_err(sql_error)?;
        let normalized_query = request.query.trim().to_lowercase();
        let mut ranked = Vec::new();
        while let Some(row) = rows.next().map_err(sql_error)? {
            let id: String = row.get(0).map_err(sql_error)?;
            let name: String = row.get(1).map_err(sql_error)?;
            let qualified: String = row.get(2).map_err(sql_error)?;
            let rank: f64 = row.get(3).map_err(sql_error)?;
            let normalized_name = name.to_lowercase();
            let normalized_qualified = qualified.to_lowercase();
            let tier = if normalized_qualified == normalized_query {
                4_u8
            } else if normalized_name == normalized_query {
                3
            } else if normalized_qualified.starts_with(&normalized_query)
                || normalized_name.starts_with(&normalized_query)
            {
                2
            } else {
                1
            };
            let mut matched_fields = Vec::new();
            if normalized_name.contains(&normalized_query) {
                matched_fields.push("name".to_owned());
            }
            if normalized_qualified.contains(&normalized_query) {
                matched_fields.push("qualified_name".to_owned());
            }
            ranked.push((tier, rank, id, matched_fields));
        }
        ranked.sort_by(|left, right| {
            right
                .0
                .cmp(&left.0)
                .then_with(|| left.1.total_cmp(&right.1))
                .then_with(|| left.2.cmp(&right.2))
        });
        let max_nodes = usize::try_from(request.limits.max_nodes).unwrap_or(usize::MAX);
        if ranked.len() > max_nodes {
            ranked.truncate(max_nodes);
            response.truncated = true;
            response.diagnostics.push(QueryDiagnostic {
                code: QueryDiagnosticCode::BoundedTruncation,
                message: format!("Search results were limited to {max_nodes} nodes"),
                node_id: None,
                path: None,
            });
        }
        for (tier, rank, id, matched_fields) in ranked {
            let Some(node) = self
                .lookup
                .node_by_id(&id)
                .map(|index| &self.graph.nodes[index])
            else {
                return Err(QueryError::new(
                    QueryErrorKind::GraphInvariant,
                    "query_graph_invariant",
                    format!("index references absent graph node {id}"),
                ));
            };
            response.results.push(SearchHit {
                node_id: id,
                score: f64::from(tier) * 1_000_000.0 - rank,
                matched_fields,
            });
            response.nodes.push(query_node(node));
        }
        if response.results.is_empty() {
            response.diagnostics.push(QueryDiagnostic {
                code: QueryDiagnosticCode::NoMatch,
                message: format!("No symbol matched {:?}", request.query),
                node_id: None,
                path: None,
            });
        }
        if self.program.is_some() {
            join_program_evidence(&mut response, self.program.as_ref());
        } else {
            response.sort_stable();
        }
        enforce_response_size(&mut response)?;
        Ok(response)
    }

    pub fn callers(&self, request: CallRequest) -> Result<CodeQueryResponse, QueryError> {
        self.call_neighbors(request, true)
    }

    pub fn callees(&self, request: CallRequest) -> Result<CodeQueryResponse, QueryError> {
        self.call_neighbors(request, false)
    }

    fn call_neighbors(
        &self,
        request: CallRequest,
        inbound: bool,
    ) -> Result<CodeQueryResponse, QueryError> {
        validate_limits(&request.limits)?;
        let operation = if inbound {
            CodeQueryOperation::Callers
        } else {
            CodeQueryOperation::Callees
        };
        let mut response = CodeQueryResponse::empty(operation, request.limits.clone());
        let Some(seed) = self.resolve_symbol(&request.symbol, &mut response) else {
            return Ok(response);
        };
        let kinds: &[EdgeKind] = if inbound {
            &[EdgeKind::Calls, EdgeKind::RoutesTo]
        } else {
            &[EdgeKind::Calls]
        };
        let max_edges = usize::try_from(request.limits.max_edges).unwrap_or(usize::MAX);
        let (selected_indices, truncated, _) =
            self.adjacency
                .matching_bounded(&self.graph, &seed, inbound, kinds, true, max_edges);
        response.truncated |= truncated;
        let selected_edges = selected_indices
            .into_iter()
            .map(|index| &self.graph.links[index])
            .collect::<Vec<_>>();
        let mut ids = HashSet::from([seed.clone()]);
        for edge in &selected_edges {
            ids.insert(edge.source.clone());
            ids.insert(edge.target.clone());
            response.edges.push(query_edge(edge));
        }
        self.add_nodes(&ids, &mut response);
        self.finish_response(&mut response)
    }

    pub fn impact(&self, request: ImpactRequest) -> Result<CodeQueryResponse, QueryError> {
        validate_limits(&request.limits)?;
        let mut response =
            CodeQueryResponse::empty(CodeQueryOperation::Impact, request.limits.clone());
        let Some(seed) = self.resolve_symbol(&request.symbol, &mut response) else {
            return Ok(response);
        };
        let max_depth = usize::try_from(request.limits.max_depth).unwrap_or(usize::MAX);
        let max_nodes = usize::try_from(request.limits.max_nodes).unwrap_or(usize::MAX);
        let mut queue =
            VecDeque::from([(seed.clone(), Vec::<String>::new(), Vec::<String>::new())]);
        let mut visited = HashSet::from([seed.clone()]);
        let max_edges = usize::try_from(request.limits.max_edges).unwrap_or(usize::MAX);
        let mut selected_edges = HashSet::new();
        while let Some((node, path_nodes, path_edges)) = queue.pop_front() {
            if path_edges.len() >= max_depth {
                continue;
            }
            let remaining_edges = max_edges.saturating_sub(selected_edges.len());
            let (incoming, incoming_truncated, _) = self.adjacency.matching_bounded(
                &self.graph,
                &node,
                true,
                IMPACT_KINDS,
                request.include_heuristic,
                remaining_edges,
            );
            response.truncated |= incoming_truncated;
            let incoming = incoming.into_iter().map(|index| &self.graph.links[index]);
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
                    response
                        .paths
                        .push(path_record(&nodes, &edges, &self.graph, &self.adjacency));
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
        self.add_nodes(&ids, &mut response);
        self.add_edges(&selected_edges, &mut response);
        self.apply_path_bound(&mut response);
        self.finish_response(&mut response)
    }

    pub fn explore(&self, request: ExploreRequest) -> Result<CodeQueryResponse, QueryError> {
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
        for symbol in &request.symbols {
            if let Some(seed) = self.resolve_symbol(symbol, &mut response) {
                seeds.push(seed);
            }
        }
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
                && let (Some((nodes, edges)), truncated) =
                    self.shortest_path(source, target, true, &request.limits, &mut budget)
            {
                response.truncated |= truncated;
                ids.extend(nodes.iter().cloned());
                edge_ids.extend(edges.iter().cloned());
                response
                    .paths
                    .push(path_record(&nodes, &edges, &self.graph, &self.adjacency));
            }
        }
        self.add_nodes(&ids, &mut response);
        self.add_edges(&edge_ids, &mut response);
        self.add_verified_files(&request.root, &mut response)?;
        self.apply_path_bound(&mut response);
        self.finish_response(&mut response)
    }

    pub fn node_trail(&self, request: NodeTrailRequest) -> Result<CodeQueryResponse, QueryError> {
        validate_limits(&request.limits)?;
        let mut response =
            CodeQueryResponse::empty(CodeQueryOperation::NodeTrail, request.limits.clone());
        let Some(source) = self.resolve_symbol(&request.source, &mut response) else {
            return Ok(response);
        };
        let Some(target) = self.resolve_symbol(&request.target, &mut response) else {
            return Ok(response);
        };
        let mut budget = TraversalBudget::new(&request.limits);
        let (path, truncated) = self.shortest_path(
            &source,
            &target,
            request.include_heuristic,
            &request.limits,
            &mut budget,
        );
        response.truncated |= truncated;
        let Some((nodes, edges)) = path else {
            if truncated {
                return self.finish_response(&mut response);
            }
            response.diagnostics.push(QueryDiagnostic {
                code: QueryDiagnosticCode::NoMatch,
                message: format!("No bounded trail connects {source} and {target}"),
                node_id: Some(source),
                path: None,
            });
            return Ok(response);
        };
        let ids = nodes.iter().cloned().collect::<HashSet<_>>();
        self.add_nodes(&ids, &mut response);
        let edge_ids = edges.iter().cloned().collect::<HashSet<_>>();
        self.add_edges(&edge_ids, &mut response);
        response
            .paths
            .push(path_record(&nodes, &edges, &self.graph, &self.adjacency));
        self.finish_response(&mut response)
    }

    #[must_use]
    pub fn graph_path(&self) -> &std::path::Path {
        &self.graph_path
    }

    #[must_use]
    pub fn index_path(&self) -> &std::path::Path {
        &self.index_path
    }

    fn resolve_symbol(&self, query: &str, response: &mut CodeQueryResponse) -> Option<String> {
        if let Some(node) = self
            .lookup
            .node_by_id(query)
            .map(|index| &self.graph.nodes[index])
        {
            return Some(node.id.clone());
        }
        let normalized = normalize_symbol(query);
        let candidate_limit = usize::try_from(response.limits.max_candidates).unwrap_or(usize::MAX);
        let exact = self
            .lookup
            .nodes_by_normalized_name(&normalized)
            .iter()
            .take(candidate_limit.saturating_add(1))
            .map(|index| self.graph.nodes[*index].id.clone())
            .collect::<Vec<_>>();
        match exact.as_slice() {
            [node] => Some(node.clone()),
            [] => {
                response.diagnostics.push(QueryDiagnostic {
                    code: QueryDiagnosticCode::NoMatch,
                    message: format!("No symbol matched {query:?}"),
                    node_id: None,
                    path: None,
                });
                None
            }
            _ => {
                response.diagnostics.push(QueryDiagnostic {
                    code: QueryDiagnosticCode::AmbiguousMatch,
                    message: format!("Symbol {query:?} matched {} nodes", exact.len()),
                    node_id: None,
                    path: None,
                });
                None
            }
        }
    }

    fn add_nodes(&self, ids: &HashSet<String>, response: &mut CodeQueryResponse) {
        let max = usize::try_from(response.limits.max_nodes).unwrap_or(usize::MAX);
        let mut nodes = ids
            .iter()
            .filter_map(|id| self.lookup.node_by_id(id))
            .map(|index| &self.graph.nodes[index])
            .collect::<Vec<_>>();
        nodes.sort_by(|left, right| left.id.cmp(&right.id));
        if nodes.len() > max {
            nodes.truncate(max);
            response.truncated = true;
        }
        response.nodes.extend(nodes.into_iter().map(query_node));
    }

    fn add_edges(&self, ids: &HashSet<String>, response: &mut CodeQueryResponse) {
        let mut edges = ids
            .iter()
            .filter_map(|id| self.adjacency.by_id(id))
            .map(|index| &self.graph.links[index])
            .collect::<Vec<_>>();
        edges.sort_by(|left, right| left.id.cmp(&right.id));
        response.edges.extend(edges.into_iter().map(query_edge));
    }

    fn shortest_path(
        &self,
        source: &str,
        target: &str,
        include_heuristic: bool,
        limits: &compass_model::query_contract::CodeQueryLimits,
        budget: &mut TraversalBudget,
    ) -> BoundedPathResult {
        let max_depth = usize::try_from(limits.max_depth).unwrap_or(usize::MAX);
        if !budget.consume_node() {
            return (None, true);
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
                        return (None, truncated);
                    };
                    edges.push(edge.clone());
                    nodes.push(previous.clone());
                    cursor = previous;
                }
                nodes.reverse();
                edges.reverse();
                return (Some((nodes, edges)), truncated);
            }
            if depth >= max_depth {
                continue;
            }
            let mut adjacent = Vec::new();
            for index in self.adjacency.incident(&node, include_heuristic) {
                if !budget.consume_edge() {
                    truncated = true;
                    break;
                }
                let edge = &self.graph.links[*index];
                if edge.source == node {
                    adjacent.push((edge.target.clone(), edge));
                } else if edge.target == node {
                    adjacent.push((edge.source.clone(), edge));
                }
            }
            adjacent.sort_by(|left, right| {
                evidence_quality(right.1)
                    .cmp(&evidence_quality(left.1))
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
        (None, truncated)
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
            let Some(record) = self
                .lookup
                .file_by_path(&path)
                .map(|index| &self.graph.graph.files[index])
            else {
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
        .is_some_and(|name| name == ".compass-generations")
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

fn path_record(
    nodes: &[String],
    edges: &[String],
    graph: &GraphDocument,
    adjacency: &CodeAdjacencyIndex,
) -> QueryPath {
    let selected = edges
        .iter()
        .filter_map(|edge| adjacency.by_id(edge))
        .map(|index| &graph.links[index])
        .collect::<Vec<_>>();
    let weakest_confidence = selected
        .iter()
        .flat_map(|edge| &edge.evidence)
        .map(|evidence| evidence.confidence)
        .max_by_key(|confidence| match confidence {
            EvidenceConfidence::Exact => 0,
            EvidenceConfidence::Inferred => 1,
            EvidenceConfidence::Ambiguous => 2,
        })
        .unwrap_or(EvidenceConfidence::Exact);
    let weakest_resolution = if weakest_confidence == EvidenceConfidence::Ambiguous {
        ResolutionState::Ambiguous
    } else if weakest_confidence == EvidenceConfidence::Inferred {
        ResolutionState::Unresolved
    } else {
        ResolutionState::Exact
    };
    QueryPath {
        id: format!("path:{}", edges.join(":")),
        node_ids: nodes.to_vec(),
        edge_ids: edges.to_vec(),
        weakest_resolution,
        weakest_confidence,
    }
}

fn normalize_symbol(value: &str) -> String {
    value
        .trim()
        .trim_end_matches("()")
        .trim_start_matches('.')
        .to_lowercase()
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
    QueryNode {
        id: node.id.clone(),
        kind: node.kind,
        roles: node.roles.clone(),
        name: node.name.clone(),
        qualified_name: node.qualified_name.clone(),
        language: node.language.clone(),
        framework: node.framework.clone(),
        source: node.source.clone(),
        details: node.details.clone(),
        evidence: node.evidence.iter().map(structural_evidence).collect(),
    }
}

pub(crate) fn query_edge(edge: &EdgeRecord) -> QueryEdge {
    QueryEdge {
        id: edge.id.clone(),
        source: edge.source.clone(),
        target: edge.target.clone(),
        kind: edge.kind,
        relationship_site: edge.relationship_site.clone(),
        details: edge.details.clone(),
        evidence: edge.evidence.iter().map(structural_evidence).collect(),
    }
}

fn structural_evidence(evidence: &compass_model::provenance::Provenance) -> QueryEvidence {
    QueryEvidence {
        layer: QueryEvidenceLayer::StructuralGraph,
        origin: evidence.origin,
        extractor: evidence.extractor.clone(),
        confidence: evidence.confidence,
        anchor: evidence.anchors.first().cloned(),
        rule: evidence.rule.clone(),
        wiring_site: evidence.wiring_site.clone(),
        resolution: if evidence.confidence == EvidenceConfidence::Ambiguous
            || evidence.candidates.len() > 1
        {
            ResolutionState::Ambiguous
        } else if evidence.confidence == EvidenceConfidence::Inferred
            && evidence.candidates.is_empty()
        {
            ResolutionState::Unresolved
        } else {
            ResolutionState::Exact
        },
        candidates: evidence.candidates.clone(),
    }
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

fn fts_query(value: &str) -> Result<String, QueryError> {
    if value.len() > 4_096 {
        return Err(QueryError::new(
            QueryErrorKind::InvalidParameter,
            "search_query_too_large",
            "search query exceeds 4096 bytes",
        ));
    }
    let terms = value
        .split(|character: char| !(character.is_alphanumeric() || character == '_'))
        .filter(|term| {
            !term.is_empty()
                && !matches!(
                    term.to_ascii_uppercase().as_str(),
                    "AND" | "OR" | "NOT" | "NEAR"
                )
        })
        .take(33)
        .collect::<Vec<_>>();
    if terms.len() > 32 {
        return Err(QueryError::new(
            QueryErrorKind::InvalidParameter,
            "too_many_search_terms",
            "search query exceeds 32 terms",
        ));
    }
    Ok(terms
        .into_iter()
        .map(|term| format!("\"{}\"*", term.replace('"', "\"\"")))
        .collect::<Vec<_>>()
        .join(" AND "))
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
        BuildMetadata, EdgeRecord, GraphDocument, NodeKind, NodeRecord,
    };
    use compass_model::provenance::{EvidenceConfidence, EvidenceOrigin, Provenance};

    use super::{CodeAdjacencyIndex, EdgeKind};

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
    fn bounded_matching_scales_with_response_budget_on_500k_edges() {
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
                evidence: Vec::new(),
                coverage: Vec::new(),
                diagnostics: Vec::new(),
                community: None,
            });
        }
        graph.links.reserve(EDGES);
        for index in 0..EDGES {
            graph.links.push(edge(
                &format!("e:{index:06}"),
                "hub",
                if index % 2 == 0 {
                    EdgeKind::Calls
                } else {
                    EdgeKind::RoutesTo
                },
                &format!("n:{:05}", index % NODES),
            ));
        }

        let adjacency = CodeAdjacencyIndex::build(&graph);
        let (matching, truncated, examined) = adjacency.matching_bounded(
            &graph,
            "hub",
            false,
            &[EdgeKind::Calls, EdgeKind::RoutesTo],
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
    }
}
