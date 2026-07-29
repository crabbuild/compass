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
    pub(crate) adjacent_edges: HashMap<String, Vec<usize>>,
}

impl CodeQueryEngine {
    pub(crate) fn build_adjacency(graph: &GraphDocument) -> HashMap<String, Vec<usize>> {
        let mut adjacent = HashMap::<String, Vec<usize>>::new();
        for (index, edge) in graph.links.iter().enumerate() {
            adjacent.entry(edge.source.clone()).or_default().push(index);
            adjacent.entry(edge.target.clone()).or_default().push(index);
        }
        for edges in adjacent.values_mut() {
            edges.sort_by(|left, right| graph.links[*left].id.cmp(&graph.links[*right].id));
        }
        adjacent
    }

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
            let Some(node) = self.graph.nodes.iter().find(|node| node.id == id) else {
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
        let mut selected_edges = self
            .graph
            .links
            .iter()
            .filter(|edge| {
                if inbound {
                    edge.target == seed && matches!(edge.kind, EdgeKind::Calls | EdgeKind::RoutesTo)
                } else {
                    edge.source == seed && edge.kind == EdgeKind::Calls
                }
            })
            .collect::<Vec<_>>();
        selected_edges.sort_by(|left, right| left.id.cmp(&right.id));
        self.bound_edges(&mut selected_edges, &mut response);
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
            let mut incoming = self
                .graph
                .links
                .iter()
                .filter(|edge| {
                    edge.target == node
                        && IMPACT_KINDS.contains(&edge.kind)
                        && (request.include_heuristic || !is_heuristic(edge))
                })
                .collect::<Vec<_>>();
            incoming.sort_by(|left, right| left.id.cmp(&right.id));
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
                        .push(path_record(&nodes, &edges, &self.graph));
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
        response.edges.extend(
            self.graph
                .links
                .iter()
                .filter(|edge| selected_edges.contains(&edge.id))
                .map(query_edge),
        );
        self.apply_path_bound(&mut response);
        self.finish_response(&mut response)
    }

    pub fn explore(&self, request: ExploreRequest) -> Result<CodeQueryResponse, QueryError> {
        validate_limits(&request.limits)?;
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
        for pair in seeds.windows(2) {
            if let [source, target] = pair
                && let (Some((nodes, edges)), truncated) =
                    self.shortest_path(source, target, true, &request.limits)
            {
                response.truncated |= truncated;
                ids.extend(nodes.iter().cloned());
                edge_ids.extend(edges.iter().cloned());
                response
                    .paths
                    .push(path_record(&nodes, &edges, &self.graph));
            }
        }
        self.add_nodes(&ids, &mut response);
        response.edges.extend(
            self.graph
                .links
                .iter()
                .filter(|edge| edge_ids.contains(&edge.id))
                .map(query_edge),
        );
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
        let (path, truncated) =
            self.shortest_path(&source, &target, request.include_heuristic, &request.limits);
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
        let edge_ids = edges.iter().collect::<HashSet<_>>();
        response.edges.extend(
            self.graph
                .links
                .iter()
                .filter(|edge| edge_ids.contains(&edge.id))
                .map(query_edge),
        );
        response
            .paths
            .push(path_record(&nodes, &edges, &self.graph));
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
        if let Some(node) = self.graph.nodes.iter().find(|node| node.id == query) {
            return Some(node.id.clone());
        }
        let normalized = normalize_symbol(query);
        let mut exact = self
            .graph
            .nodes
            .iter()
            .filter(|node| {
                normalize_symbol(&node.name) == normalized
                    || normalize_symbol(&node.qualified_name) == normalized
            })
            .map(|node| node.id.clone())
            .collect::<Vec<_>>();
        exact.sort();
        exact.dedup();
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
        let mut nodes = self
            .graph
            .nodes
            .iter()
            .filter(|node| ids.contains(&node.id))
            .collect::<Vec<_>>();
        nodes.sort_by(|left, right| left.id.cmp(&right.id));
        if nodes.len() > max {
            nodes.truncate(max);
            response.truncated = true;
        }
        response.nodes.extend(nodes.into_iter().map(query_node));
    }

    fn bound_edges(&self, edges: &mut Vec<&EdgeRecord>, response: &mut CodeQueryResponse) {
        let max = usize::try_from(response.limits.max_edges).unwrap_or(usize::MAX);
        if edges.len() > max {
            edges.truncate(max);
            response.truncated = true;
        }
    }

    fn shortest_path(
        &self,
        source: &str,
        target: &str,
        include_heuristic: bool,
        limits: &compass_model::query_contract::CodeQueryLimits,
    ) -> (Option<(Vec<String>, Vec<String>)>, bool) {
        let max_depth = usize::try_from(limits.max_depth).unwrap_or(usize::MAX);
        let max_nodes = usize::try_from(limits.max_nodes).unwrap_or(usize::MAX);
        let max_edges = usize::try_from(limits.max_edges).unwrap_or(usize::MAX);
        let mut queue = VecDeque::from([(source.to_owned(), 0_usize)]);
        let mut visited = HashSet::from([source.to_owned()]);
        let mut predecessor = HashMap::<String, (String, String)>::new();
        let mut traversed_edges = 0_usize;
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
            let mut adjacent = self
                .adjacent_edges
                .get(&node)
                .into_iter()
                .flatten()
                .filter_map(|index| {
                    let edge = &self.graph.links[*index];
                    if !include_heuristic && is_heuristic(edge) {
                        return None;
                    }
                    if edge.source == node {
                        Some((edge.target.clone(), edge))
                    } else if edge.target == node {
                        Some((edge.source.clone(), edge))
                    } else {
                        None
                    }
                })
                .collect::<Vec<_>>();
            adjacent.sort_by(|left, right| {
                evidence_quality(right.1)
                    .cmp(&evidence_quality(left.1))
                    .then_with(|| left.1.id.cmp(&right.1.id))
            });
            for (next, edge) in adjacent {
                if visited.contains(&next) {
                    continue;
                }
                if visited.len() >= max_nodes || traversed_edges >= max_edges {
                    truncated = true;
                    continue;
                }
                visited.insert(next.clone());
                traversed_edges += 1;
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
            self.graph_path
                .parent()
                .and_then(Path::parent)
                .unwrap_or_else(|| Path::new("."))
                .to_path_buf()
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
            let Some(record) = self.graph.graph.files.iter().find(|file| file.path == path) else {
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
        if response.truncated {
            response.diagnostics.push(QueryDiagnostic {
                code: QueryDiagnosticCode::BoundedTruncation,
                message: "One or more query bounds truncated the response".to_owned(),
                node_id: None,
                path: None,
            });
        }
        if self.program.is_some() {
            join_program_evidence(response, self.program.as_ref());
        } else {
            response.sort_stable();
        }
        enforce_response_size(response)?;
        Ok(response.clone())
    }
}

fn path_record(nodes: &[String], edges: &[String], graph: &GraphDocument) -> QueryPath {
    let selected = graph
        .links
        .iter()
        .filter(|edge| edges.contains(&edge.id))
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
