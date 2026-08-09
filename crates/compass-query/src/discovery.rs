use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use compass_model::code_graph::{EdgeKind, EdgeRecord, NodeRecord};
use compass_model::provenance::EvidenceOrigin;
use compass_model::query_contract::{
    DISCOVERY_QUERY_SCHEMA_V1, DiscoveryAlternative, DiscoveryDirection, DiscoveryDirectionSource,
    DiscoveryLimits, DiscoveryOmissions, DiscoveryQueryRequest, DiscoveryQueryResponse,
    DiscoveryScope, DiscoveryScopeKind, DiscoveryScoreTier, DiscoverySeed, DiscoverySeedSource,
    DiscoveryStats, MAX_DISCOVERY_FILTER_BYTES, MAX_DISCOVERY_FILTERS,
    MAX_DISCOVERY_QUESTION_BYTES, QueryDiagnostic, QueryDiagnosticCode,
};

use crate::code_query::{query_edge, query_node};
use crate::ranking::rank_search_candidates;
use crate::recall::{CandidateSource, RecallBudget, SearchCandidatePool};
use crate::text::{normalize_context_filters, query_terms, search_tokens, strip_diacritics};
use crate::{CodeQueryEngine, QueryError, QueryErrorKind};

const ALL_EDGE_KINDS: &[EdgeKind] = &[
    EdgeKind::Contains,
    EdgeKind::Embeds,
    EdgeKind::Calls,
    EdgeKind::Imports,
    EdgeKind::Exports,
    EdgeKind::Extends,
    EdgeKind::Implements,
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
];

#[derive(Clone, Debug)]
struct RankedReferenceCandidate {
    node: NodeRecord,
    score: f64,
    matched_terms: Vec<String>,
    matched_fields: Vec<String>,
    source: DiscoverySeedSource,
}

struct DiscoveryGuard<'a> {
    deadline: Instant,
    cancelled: Option<&'a AtomicBool>,
}

impl<'a> DiscoveryGuard<'a> {
    fn new(timeout_ms: u64, cancelled: Option<&'a AtomicBool>) -> Self {
        let started = Instant::now();
        Self {
            deadline: started + Duration::from_millis(timeout_ms),
            cancelled,
        }
    }

    fn check(&self) -> Result<(), QueryError> {
        if self
            .cancelled
            .is_some_and(|cancelled| cancelled.load(Ordering::Relaxed))
        {
            return Err(QueryError::new(
                QueryErrorKind::Cancelled,
                "discovery_cancelled",
                "discovery query was cancelled",
            ));
        }
        if Instant::now() >= self.deadline {
            return Err(QueryError::new(
                QueryErrorKind::Timeout,
                "discovery_timeout",
                "discovery query exceeded its timeout",
            ));
        }
        Ok(())
    }
}

impl CodeQueryEngine {
    /// Discover likely code-graph seeds and a bounded structural neighborhood.
    pub fn discover(
        &self,
        request: DiscoveryQueryRequest,
    ) -> Result<DiscoveryQueryResponse, QueryError> {
        self.discover_with_cancellation(request, None)
    }

    /// Execute discovery while observing an optional caller-owned cancellation flag.
    pub fn discover_with_cancellation(
        &self,
        request: DiscoveryQueryRequest,
        cancelled: Option<&AtomicBool>,
    ) -> Result<DiscoveryQueryResponse, QueryError> {
        validate_request(&request)?;
        let guard = DiscoveryGuard::new(request.limits.timeout_ms, cancelled);
        guard.check()?;

        let relation_contexts = normalize_context_filters(&request.relation_contexts);
        let selected_direction = match request.direction {
            DiscoveryDirection::Auto => DiscoveryDirection::Both,
            direction => direction,
        };
        let mut response = DiscoveryQueryResponse {
            schema: DISCOVERY_QUERY_SCHEMA_V1.to_owned(),
            question: request.question.clone(),
            selected_direction,
            direction_source: if request.direction == DiscoveryDirection::Auto {
                DiscoveryDirectionSource::Neutral
            } else {
                DiscoveryDirectionSource::Explicit
            },
            relation_contexts,
            scope: canonical_scope(&request.scope),
            traversal: request.traversal,
            seeds: Vec::new(),
            nodes: Vec::new(),
            edges: Vec::new(),
            diagnostics: Vec::new(),
            limits: request.limits.clone(),
            stats: DiscoveryStats::default(),
            omissions: DiscoveryOmissions::default(),
            truncated: false,
        };

        let (mut candidates, candidate_truncated) =
            self.reference_candidates(&request.question, &response.scope, &request.limits, &guard)?;
        response.stats.candidate_nodes = u64::try_from(candidates.1).unwrap_or(u64::MAX);
        response.stats.candidates_admitted = u64::try_from(candidates.0.len()).unwrap_or(u64::MAX);
        if candidate_truncated {
            response.truncated = true;
        }

        let max_seeds = usize::try_from(request.limits.max_seeds)
            .unwrap_or(usize::MAX)
            .min(usize::try_from(request.limits.max_nodes).unwrap_or(usize::MAX));
        candidates.0.truncate(max_seeds);
        response.seeds = discovery_seeds(&candidates.0);
        if response.seeds.is_empty() {
            response.diagnostics.push(QueryDiagnostic {
                code: QueryDiagnosticCode::NoMatch,
                message: format!("No node matched {:?}", request.question),
                node_id: None,
                path: None,
            });
            finish_response(&guard, &mut response)?;
            return Ok(response);
        }

        self.expand_reference_neighborhood(&request, &guard, &mut response)?;
        if let Some(message) = &self.partial_graph_message {
            response.diagnostics.push(QueryDiagnostic {
                code: QueryDiagnosticCode::IncompleteCoverage,
                message: message.clone(),
                node_id: None,
                path: None,
            });
        }
        finish_response(&guard, &mut response)?;
        Ok(response)
    }

    fn reference_candidates(
        &self,
        question: &str,
        scope: &[DiscoveryScope],
        limits: &DiscoveryLimits,
        guard: &DiscoveryGuard<'_>,
    ) -> Result<((Vec<RankedReferenceCandidate>, usize), bool), QueryError> {
        let max_candidates = usize::try_from(limits.max_candidates).unwrap_or(usize::MAX);
        let (mut nodes, truncated) = self.backend.reference_nodes_bounded(max_candidates)?;
        nodes.sort_by(|left, right| left.id.cmp(&right.id));
        let examined = nodes.len();
        let terms = query_terms(question);
        let mut pool = SearchCandidatePool::new(RecallBudget {
            max_total_candidates: max_candidates,
            max_per_source: max_candidates,
            max_fuzzy_candidates: 0,
        });
        for node in nodes {
            guard.check()?;
            if !scope_matches(&node, scope) {
                continue;
            }
            if let Some(source) = reference_candidate_source(question, &terms, &node) {
                let _ = pool.add(source, node);
            }
        }
        let candidates = rank_search_candidates(question, &terms, pool.into_vec(), max_candidates)
            .into_iter()
            .map(|ranked| {
                let source = discovery_candidate_source(question, &ranked.node);
                RankedReferenceCandidate {
                    matched_terms: matched_terms(&terms, &ranked.node),
                    matched_fields: ranked.matched_fields,
                    node: ranked.node,
                    score: ranked.score,
                    source,
                }
            })
            .collect();
        Ok(((candidates, examined), truncated))
    }

    fn expand_reference_neighborhood(
        &self,
        request: &DiscoveryQueryRequest,
        guard: &DiscoveryGuard<'_>,
        response: &mut DiscoveryQueryResponse,
    ) -> Result<(), QueryError> {
        let max_nodes = usize::try_from(request.limits.max_nodes).unwrap_or(usize::MAX);
        let max_edges = usize::try_from(request.limits.max_edges).unwrap_or(usize::MAX);
        let max_expanded = request.limits.max_expanded_relationships;
        let max_depth = usize::try_from(request.limits.max_depth).unwrap_or(usize::MAX);
        let mut selected_nodes = BTreeMap::<String, NodeRecord>::new();
        let mut selected_edges = BTreeMap::<String, EdgeRecord>::new();
        let mut selected_relationships = BTreeSet::<(String, String, String)>::new();
        let mut omitted_node_ids = BTreeSet::new();
        let mut omitted_edge_ids = BTreeSet::new();
        let mut visited = BTreeSet::new();
        let mut frontier = VecDeque::new();

        for seed in &response.seeds {
            guard.check()?;
            let Some(node) = self.backend.node_by_id(&seed.node_id)? else {
                return Err(QueryError::new(
                    QueryErrorKind::GraphInvariant,
                    "discovery_seed_missing",
                    format!("discovery seed {} is absent from the graph", seed.node_id),
                ));
            };
            visited.insert(node.id.clone());
            frontier.push_back((node.id.clone(), 0_usize));
            selected_nodes.insert(node.id.clone(), node);
        }

        while let Some((node_id, depth)) = match request.traversal {
            compass_model::query_contract::DiscoveryTraversal::Bfs => frontier.pop_front(),
            compass_model::query_contract::DiscoveryTraversal::Dfs => frontier.pop_back(),
        } {
            guard.check()?;
            if depth >= max_depth || response.stats.expanded_relationships >= max_expanded {
                if response.stats.expanded_relationships >= max_expanded && !frontier.is_empty() {
                    mark_expansion_truncated(response);
                }
                continue;
            }
            let remaining =
                usize::try_from(max_expanded.saturating_sub(response.stats.expanded_relationships))
                    .unwrap_or(usize::MAX);
            let (edges, truncated) = edges_for_direction(
                &self.backend,
                &node_id,
                response.selected_direction,
                request.include_heuristic,
                remaining,
            )?;
            if truncated {
                mark_expansion_truncated(response);
            }
            for edge in edges {
                guard.check()?;
                response.stats.expanded_relationships =
                    response.stats.expanded_relationships.saturating_add(1);
                if response.stats.expanded_relationships > max_expanded {
                    mark_expansion_truncated(response);
                    break;
                }
                if !edge_matches_context(&edge, &response.relation_contexts)
                    || (!request.include_heuristic && is_heuristic(&edge))
                {
                    continue;
                }
                let other_id = if edge.source == node_id {
                    edge.target.clone()
                } else {
                    edge.source.clone()
                };
                let Some(other) = self.backend.node_by_id(&other_id)? else {
                    return Err(QueryError::new(
                        QueryErrorKind::GraphInvariant,
                        "discovery_edge_endpoint_missing",
                        format!("edge {} references absent node {other_id}", edge.id),
                    ));
                };
                if !scope_matches(&other, &response.scope) {
                    continue;
                }
                if selected_nodes.len() >= max_nodes && !selected_nodes.contains_key(&other_id) {
                    response.truncated = true;
                    omitted_node_ids.insert(other_id);
                    continue;
                }
                let relationship = (
                    edge.source.clone(),
                    edge.target.clone(),
                    edge.kind.as_str().to_owned(),
                );
                if selected_edges.len() >= max_edges
                    && !selected_relationships.contains(&relationship)
                {
                    response.truncated = true;
                    omitted_edge_ids.insert(edge.id.clone());
                    continue;
                }
                selected_nodes.entry(other.id.clone()).or_insert(other);
                if selected_relationships.insert(relationship) {
                    selected_edges.insert(edge.id.clone(), edge);
                }
                if visited.insert(other_id.clone()) {
                    frontier.push_back((other_id, depth.saturating_add(1)));
                }
            }
        }

        response.nodes = selected_nodes.values().map(query_node).collect();
        response.edges = selected_edges.values().map(query_edge).collect();
        if !omitted_node_ids.is_empty() {
            response.omissions.nodes =
                Some(u64::try_from(omitted_node_ids.len()).unwrap_or(u64::MAX));
        }
        if !omitted_edge_ids.is_empty() {
            response.omissions.edges =
                Some(u64::try_from(omitted_edge_ids.len()).unwrap_or(u64::MAX));
        }
        response.stats.visited_nodes = u64::try_from(visited.len()).unwrap_or(u64::MAX);
        Ok(())
    }
}

fn validate_request(request: &DiscoveryQueryRequest) -> Result<(), QueryError> {
    if !request.limits.is_valid() {
        return Err(QueryError::new(
            QueryErrorKind::InvalidParameter,
            "invalid_discovery_limits",
            "discovery limits must be positive and no greater than their hard ceilings",
        ));
    }
    if request.question.trim().is_empty() || request.question.len() > MAX_DISCOVERY_QUESTION_BYTES {
        return Err(QueryError::new(
            QueryErrorKind::InvalidParameter,
            "invalid_discovery_question",
            format!("discovery question must contain 1 to {MAX_DISCOVERY_QUESTION_BYTES} bytes"),
        ));
    }
    validate_filters("relationContexts", &request.relation_contexts)?;
    if request.scope.len() > MAX_DISCOVERY_FILTERS
        || request
            .scope
            .iter()
            .any(|scope| scope.value.is_empty() || scope.value.len() > MAX_DISCOVERY_FILTER_BYTES)
    {
        return Err(QueryError::new(
            QueryErrorKind::InvalidParameter,
            "invalid_discovery_scope",
            format!(
                "scope accepts at most {MAX_DISCOVERY_FILTERS} non-empty values of at most {MAX_DISCOVERY_FILTER_BYTES} bytes"
            ),
        ));
    }
    Ok(())
}

fn validate_filters(label: &str, filters: &[String]) -> Result<(), QueryError> {
    if filters.len() > MAX_DISCOVERY_FILTERS
        || filters
            .iter()
            .any(|value| value.is_empty() || value.len() > MAX_DISCOVERY_FILTER_BYTES)
    {
        return Err(QueryError::new(
            QueryErrorKind::InvalidParameter,
            "invalid_discovery_filter",
            format!(
                "{label} accepts at most {MAX_DISCOVERY_FILTERS} non-empty values of at most {MAX_DISCOVERY_FILTER_BYTES} bytes"
            ),
        ));
    }
    Ok(())
}

fn canonical_scope(scope: &[DiscoveryScope]) -> Vec<DiscoveryScope> {
    let mut scope = scope.to_vec();
    scope.sort_by(|left, right| {
        scope_kind_rank(left.kind)
            .cmp(&scope_kind_rank(right.kind))
            .then_with(|| left.value.cmp(&right.value))
    });
    scope.dedup();
    scope
}

const fn scope_kind_rank(kind: DiscoveryScopeKind) -> u8 {
    match kind {
        DiscoveryScopeKind::Community => 0,
        DiscoveryScopeKind::Source => 1,
        DiscoveryScopeKind::Package => 2,
        DiscoveryScopeKind::Node => 3,
    }
}

fn scope_matches(node: &NodeRecord, scope: &[DiscoveryScope]) -> bool {
    scope.is_empty()
        || scope.iter().any(|entry| match entry.kind {
            DiscoveryScopeKind::Community => node.community.as_ref().is_some_and(|community| {
                community.id.to_string() == entry.value
                    || community.label.as_deref() == Some(entry.value.as_str())
            }),
            DiscoveryScopeKind::Source => node.source.as_ref().is_some_and(|source| {
                source.file == entry.value
                    || source
                        .file
                        .strip_prefix(&entry.value)
                        .is_some_and(|suffix| suffix.starts_with('/'))
            }),
            DiscoveryScopeKind::Package => {
                node.qualified_name == entry.value
                    || node
                        .qualified_name
                        .strip_prefix(&entry.value)
                        .is_some_and(|suffix| {
                            suffix.starts_with("::")
                                || suffix.starts_with('.')
                                || suffix.starts_with('/')
                        })
                    || node.source.as_ref().is_some_and(|source| {
                        source
                            .file
                            .strip_prefix(&entry.value)
                            .is_some_and(|suffix| suffix.starts_with('/'))
                    })
            }
            DiscoveryScopeKind::Node => {
                node.id == entry.value || node.qualified_name == entry.value
            }
        })
}

fn reference_candidate_source(
    question: &str,
    terms: &[String],
    node: &NodeRecord,
) -> Option<CandidateSource> {
    let normalized_question = normalize_discovery_text(question);
    if node.id.to_lowercase() == normalized_question {
        return Some(CandidateSource::ExactId);
    }
    if normalize_discovery_text(&node.name) == normalized_question
        || normalize_discovery_text(&node.qualified_name) == normalized_question
    {
        return Some(CandidateSource::ExactName);
    }
    (!matched_terms(terms, node).is_empty()).then_some(CandidateSource::HeuristicFallback)
}

fn discovery_candidate_source(question: &str, node: &NodeRecord) -> DiscoverySeedSource {
    let normalized_question = normalize_discovery_text(question);
    if node.id.to_lowercase() == normalized_question {
        DiscoverySeedSource::ExactId
    } else if normalize_discovery_text(&node.name) == normalized_question
        || normalize_discovery_text(&node.qualified_name) == normalized_question
    {
        DiscoverySeedSource::ExactName
    } else {
        DiscoverySeedSource::ReferenceScan
    }
}

fn matched_terms(terms: &[String], node: &NodeRecord) -> Vec<String> {
    let mut text = search_tokens(&node.name);
    text.extend(search_tokens(&node.qualified_name));
    if let Some(source) = &node.source {
        text.extend(search_tokens(&source.file));
    }
    terms
        .iter()
        .filter(|term| {
            text.iter()
                .any(|token| token == *term || token.contains(*term))
        })
        .cloned()
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect()
}

fn normalize_discovery_text(value: &str) -> String {
    strip_diacritics(value)
        .trim()
        .trim_end_matches("()")
        .trim_start_matches('.')
        .to_lowercase()
}

fn discovery_seeds(candidates: &[RankedReferenceCandidate]) -> Vec<DiscoverySeed> {
    candidates
        .iter()
        .enumerate()
        .map(|(index, candidate)| {
            let alternatives = candidates
                .iter()
                .filter(|other| {
                    other.score.total_cmp(&candidate.score).is_eq()
                        && other.node.id != candidate.node.id
                })
                .map(|other| DiscoveryAlternative {
                    node_id: other.node.id.clone(),
                    qualified_name: other.node.qualified_name.clone(),
                    source: other.node.source.clone(),
                    score: format_discovery_score(other.score),
                })
                .collect::<Vec<_>>();
            DiscoverySeed {
                node_id: candidate.node.id.clone(),
                score: format_discovery_score(candidate.score),
                score_tier: match candidate.source {
                    DiscoverySeedSource::ExactId => DiscoveryScoreTier::ExactId,
                    DiscoverySeedSource::ExactName => DiscoveryScoreTier::ExactName,
                    DiscoverySeedSource::ReferenceScan
                    | DiscoverySeedSource::TermIndex
                    | DiscoverySeedSource::Fuzzy => DiscoveryScoreTier::Lexical,
                },
                rank: u32::try_from(index.saturating_add(1)).unwrap_or(u32::MAX),
                matched_terms: candidate.matched_terms.clone(),
                matched_fields: candidate.matched_fields.clone(),
                source: candidate.node.source.clone(),
                candidate_source: candidate.source,
                ambiguous: !alternatives.is_empty(),
                alternatives,
            }
        })
        .collect()
}

fn format_discovery_score(score: f64) -> String {
    format!("{score:.6}")
}

fn edges_for_direction(
    backend: &crate::code_query::CodeGraphBackend,
    node: &str,
    direction: DiscoveryDirection,
    include_heuristic: bool,
    limit: usize,
) -> Result<(Vec<EdgeRecord>, bool), QueryError> {
    match direction {
        DiscoveryDirection::Incoming => {
            backend.matching_bounded(node, true, ALL_EDGE_KINDS, include_heuristic, limit)
        }
        DiscoveryDirection::Outgoing => {
            backend.matching_bounded(node, false, ALL_EDGE_KINDS, include_heuristic, limit)
        }
        DiscoveryDirection::Auto | DiscoveryDirection::Both => {
            backend.incident_bounded(node, include_heuristic, limit)
        }
    }
}

fn edge_matches_context(edge: &EdgeRecord, contexts: &[String]) -> bool {
    contexts.is_empty()
        || edge
            .context
            .as_ref()
            .is_some_and(|context| contexts.iter().any(|candidate| candidate == context))
}

fn is_heuristic(edge: &EdgeRecord) -> bool {
    edge.evidence
        .iter()
        .any(|evidence| evidence.origin == EvidenceOrigin::Heuristic)
}

fn mark_expansion_truncated(response: &mut DiscoveryQueryResponse) {
    response.truncated = true;
}

fn finish_response(
    guard: &DiscoveryGuard<'_>,
    response: &mut DiscoveryQueryResponse,
) -> Result<(), QueryError> {
    guard.check()?;
    response.seeds.sort_by_key(|seed| seed.rank);
    response.nodes.sort_by(|left, right| left.id.cmp(&right.id));
    response.edges.sort_by(|left, right| left.id.cmp(&right.id));
    response.diagnostics.sort_by(|left, right| {
        left.code
            .cmp(&right.code)
            .then_with(|| left.message.cmp(&right.message))
            .then_with(|| left.node_id.cmp(&right.node_id))
    });
    ensure_truncation_diagnostic(response);
    enforce_response_bytes(response)?;
    Ok(())
}

fn enforce_response_bytes(response: &mut DiscoveryQueryResponse) -> Result<(), QueryError> {
    let max_bytes = usize::try_from(response.limits.max_response_bytes).unwrap_or(usize::MAX);
    loop {
        response.stats.returned_nodes = u64::try_from(response.nodes.len()).unwrap_or(u64::MAX);
        response.stats.returned_edges = u64::try_from(response.edges.len()).unwrap_or(u64::MAX);
        ensure_truncation_diagnostic(response);
        response.diagnostics.sort_by(|left, right| {
            left.code
                .cmp(&right.code)
                .then_with(|| left.message.cmp(&right.message))
                .then_with(|| left.node_id.cmp(&right.node_id))
        });
        let bytes = serde_json::to_vec(response).map_err(|error| {
            QueryError::new(
                QueryErrorKind::Internal,
                "discovery_response_encode_failed",
                error.to_string(),
            )
        })?;
        if bytes.len() <= max_bytes {
            return Ok(());
        }
        response.truncated = true;
        if response.edges.pop().is_some() {
            add_known_omission(&mut response.omissions.edges, 1);
            continue;
        }
        let seed_ids = response
            .seeds
            .iter()
            .map(|seed| seed.node_id.as_str())
            .collect::<BTreeSet<_>>();
        if let Some(index) = response
            .nodes
            .iter()
            .rposition(|node| !seed_ids.contains(node.id.as_str()))
        {
            let removed = response.nodes.remove(index);
            let retained_edges = response.edges.len();
            response
                .edges
                .retain(|edge| edge.source != removed.id && edge.target != removed.id);
            add_known_omission(
                &mut response.omissions.edges,
                u64::try_from(retained_edges.saturating_sub(response.edges.len()))
                    .unwrap_or(u64::MAX),
            );
            add_known_omission(&mut response.omissions.nodes, 1);
            continue;
        }
        return Err(QueryError::new(
            QueryErrorKind::MemoryLimit,
            "discovery_response_limit",
            format!("the minimal coherent discovery response exceeds maxResponseBytes {max_bytes}"),
        ));
    }
}

fn ensure_truncation_diagnostic(response: &mut DiscoveryQueryResponse) {
    if response.truncated
        && !response
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.code == QueryDiagnosticCode::BoundedTruncation)
    {
        response.diagnostics.push(QueryDiagnostic {
            code: QueryDiagnosticCode::BoundedTruncation,
            message: "One or more discovery bounds truncated the response".to_owned(),
            node_id: None,
            path: None,
        });
    }
}

fn add_known_omission(slot: &mut Option<u64>, count: u64) {
    *slot = Some(slot.unwrap_or(0).saturating_add(count));
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;
    use std::sync::atomic::AtomicBool;

    use compass_model::code_graph::{
        BuildMetadata, EdgeKind, EdgeRecord, GraphDocument, NodeKind, NodeRecord,
    };
    use compass_model::query_contract::{
        DiscoveryDirection, DiscoveryDirectionSource, DiscoveryLimits, DiscoveryQueryRequest,
        DiscoveryScope, DiscoveryScopeKind, DiscoveryTraversal, MAX_DISCOVERY_FILTER_BYTES,
        MAX_DISCOVERY_FILTERS, MAX_DISCOVERY_QUESTION_BYTES, QueryDiagnosticCode,
    };

    use crate::code_query::{
        CodeAdjacencyIndex, CodeGraphBackend, CodeLookupIndex, FuzzyLookupCache, SearchQueryCache,
    };
    use crate::{CodeQueryEngine, QueryEngineKind};

    fn node(id: &str, name: &str) -> NodeRecord {
        NodeRecord {
            id: id.to_owned(),
            kind: NodeKind::Function,
            roles: Vec::new(),
            name: name.to_owned(),
            qualified_name: format!("example::{name}"),
            language: Some("rust".to_owned()),
            framework: None,
            source: None,
            details: None,
            evidence: Vec::new(),
            coverage: Vec::new(),
            diagnostics: Vec::new(),
            community: None,
        }
    }

    fn edge(id: &str, source: &str, target: &str) -> EdgeRecord {
        EdgeRecord {
            id: id.to_owned(),
            key: id.to_owned(),
            source: source.to_owned(),
            target: target.to_owned(),
            kind: EdgeKind::Calls,
            occurrence_rule: None,
            relationship_site: None,
            details: None,
            evidence: Vec::new(),
            weight: None,
            context: Some("call".to_owned()),
            deferred: false,
            diagnostics: Vec::new(),
        }
    }

    fn engine(nodes: Vec<NodeRecord>, edges: Vec<EdgeRecord>) -> CodeQueryEngine {
        let mut graph = GraphDocument::empty_v1(BuildMetadata {
            builder_version: "test".to_owned(),
            schema_fingerprint: "schema".to_owned(),
            source_tree_digest: "tree".to_owned(),
            configuration_digest: "config".to_owned(),
            generation_id: "generation".to_owned(),
            source_commit: None,
        });
        graph.nodes = nodes;
        graph.nodes.sort_by(|left, right| left.id.cmp(&right.id));
        graph.links = edges;
        graph.links.sort_by(|left, right| left.id.cmp(&right.id));
        let adjacency = CodeAdjacencyIndex::build(&graph);
        let lookup = CodeLookupIndex::build(&graph);
        CodeQueryEngine {
            backend: CodeGraphBackend::Materialized {
                graph: Box::new(graph),
                adjacency: Box::new(adjacency),
                lookup: Box::new(lookup),
            },
            program: None,
            connection: None,
            graph_path: PathBuf::from("graph.json"),
            index_path: PathBuf::from("index.sqlite3"),
            partial_graph_message: None,
            engine_kind: QueryEngineKind::Json,
            search_query_cache: std::sync::Mutex::new(SearchQueryCache::default()),
            fuzzy_lookup_cache: std::sync::Mutex::new(FuzzyLookupCache::default()),
        }
    }

    fn request(direction: DiscoveryDirection) -> DiscoveryQueryRequest {
        DiscoveryQueryRequest {
            question: "alpha".to_owned(),
            direction,
            relation_contexts: Vec::new(),
            scope: Vec::new(),
            traversal: DiscoveryTraversal::Bfs,
            include_heuristic: false,
            limits: DiscoveryLimits::default(),
        }
    }

    #[test]
    fn explicit_direction_is_identified_as_explicit() -> Result<(), Box<dyn std::error::Error>> {
        let engine = engine(vec![node("n:alpha", "alpha")], Vec::new());
        for direction in [
            DiscoveryDirection::Incoming,
            DiscoveryDirection::Outgoing,
            DiscoveryDirection::Both,
        ] {
            let response = engine.discover(request(direction))?;
            assert_eq!(response.selected_direction, direction);
            assert_eq!(
                response.direction_source,
                DiscoveryDirectionSource::Explicit
            );
        }
        let response = engine.discover(request(DiscoveryDirection::Auto))?;
        assert_eq!(response.selected_direction, DiscoveryDirection::Both);
        assert_eq!(response.direction_source, DiscoveryDirectionSource::Neutral);
        Ok(())
    }

    #[test]
    fn returned_stats_are_inside_the_final_byte_bound() -> Result<(), Box<dyn std::error::Error>> {
        let mut nodes = vec![node("n:alpha", "alpha")];
        let mut edges = Vec::new();
        for index in 0..12 {
            let id = format!("n:neighbor:{index:02}");
            nodes.push(node(&id, &format!("neighbor_{index:02}")));
            edges.push(edge(&format!("e:{index:02}"), "n:alpha", &id));
        }
        let engine = engine(nodes, edges);
        let full = engine.discover(request(DiscoveryDirection::Both))?;
        let full_bytes = serde_json::to_vec(&full)?.len();
        let mut bounded_request = request(DiscoveryDirection::Both);
        bounded_request.limits.max_response_bytes =
            u64::try_from(full_bytes.saturating_sub(100)).unwrap_or(u64::MAX);
        let response = engine.discover(bounded_request)?;
        let bytes = serde_json::to_vec(&response)?;
        assert!(bytes.len() as u64 <= response.limits.max_response_bytes);
        assert_eq!(response.stats.returned_nodes, response.nodes.len() as u64);
        assert_eq!(response.stats.returned_edges, response.edges.len() as u64);
        let node_ids = response
            .nodes
            .iter()
            .map(|node| node.id.as_str())
            .collect::<std::collections::BTreeSet<_>>();
        assert!(response.edges.iter().all(|edge| {
            node_ids.contains(edge.source.as_str()) && node_ids.contains(edge.target.as_str())
        }));
        Ok(())
    }

    #[test]
    fn byte_truncation_always_has_a_bounded_diagnostic() -> Result<(), Box<dyn std::error::Error>> {
        let mut nodes = vec![node("n:alpha", "alpha")];
        let mut edges = Vec::new();
        for index in 0..8 {
            let id = format!("n:neighbor:{index:02}");
            nodes.push(node(&id, &format!("neighbor_{index:02}")));
            edges.push(edge(&format!("e:{index:02}"), "n:alpha", &id));
        }
        let engine = engine(nodes, edges);
        let full = engine.discover(request(DiscoveryDirection::Both))?;
        let mut bounded_request = request(DiscoveryDirection::Both);
        bounded_request.limits.max_response_bytes =
            u64::try_from(serde_json::to_vec(&full)?.len().saturating_sub(100)).unwrap_or(u64::MAX);
        let response = engine.discover(bounded_request)?;
        assert!(response.truncated);
        assert!(
            response
                .diagnostics
                .iter()
                .any(|diagnostic| { diagnostic.code == QueryDiagnosticCode::BoundedTruncation })
        );
        Ok(())
    }

    #[test]
    fn node_omissions_count_unique_identities() -> Result<(), Box<dyn std::error::Error>> {
        let engine = engine(
            vec![node("n:alpha", "alpha"), node("n:other", "other")],
            vec![
                edge("e:first", "n:alpha", "n:other"),
                edge("e:second", "n:alpha", "n:other"),
            ],
        );
        let mut bounded_request = request(DiscoveryDirection::Both);
        bounded_request.limits.max_nodes = 1;
        let response = engine.discover(bounded_request)?;
        assert_eq!(response.omissions.nodes, Some(1));
        Ok(())
    }

    #[test]
    fn cancellation_is_typed_and_checked_before_work() {
        let engine = engine(vec![node("n:alpha", "alpha")], Vec::new());
        let cancelled = AtomicBool::new(true);
        let error = match engine
            .discover_with_cancellation(request(DiscoveryDirection::Auto), Some(&cancelled))
        {
            Ok(_) => std::process::abort(),
            Err(error) => error,
        };
        assert_eq!(error.kind(), crate::QueryErrorKind::Cancelled);
        assert_eq!(error.code(), "discovery_cancelled");
    }

    #[test]
    fn no_match_is_structured_and_deterministic() -> Result<(), Box<dyn std::error::Error>> {
        let engine = engine(vec![node("n:beta", "beta")], Vec::new());
        let first = engine.discover(request(DiscoveryDirection::Auto))?;
        let second = engine.discover(request(DiscoveryDirection::Auto))?;
        assert_eq!(serde_json::to_vec(&first)?, serde_json::to_vec(&second)?);
        assert!(first.seeds.is_empty());
        assert!(first.nodes.is_empty());
        assert!(first.edges.is_empty());
        assert!(
            first
                .diagnostics
                .iter()
                .any(|diagnostic| diagnostic.code == QueryDiagnosticCode::NoMatch)
        );
        Ok(())
    }

    #[test]
    fn timeout_is_a_typed_error() {
        let guard = super::DiscoveryGuard {
            deadline: std::time::Instant::now(),
            cancelled: None,
        };
        let error = match guard.check() {
            Ok(()) => std::process::abort(),
            Err(error) => error,
        };
        assert_eq!(error.kind(), crate::QueryErrorKind::Timeout);
        assert_eq!(error.code(), "discovery_timeout");
    }

    #[test]
    fn question_and_filter_bounds_are_rejected_table_driven() {
        let base = request(DiscoveryDirection::Auto);
        let invalid = [
            DiscoveryQueryRequest {
                question: String::new(),
                ..base.clone()
            },
            DiscoveryQueryRequest {
                question: "q".repeat(MAX_DISCOVERY_QUESTION_BYTES + 1),
                ..base.clone()
            },
            DiscoveryQueryRequest {
                relation_contexts: vec!["call".to_owned(); MAX_DISCOVERY_FILTERS + 1],
                ..base.clone()
            },
            DiscoveryQueryRequest {
                relation_contexts: vec![String::new()],
                ..base.clone()
            },
            DiscoveryQueryRequest {
                relation_contexts: vec!["x".repeat(MAX_DISCOVERY_FILTER_BYTES + 1)],
                ..base.clone()
            },
            DiscoveryQueryRequest {
                scope: vec![
                    DiscoveryScope {
                        kind: DiscoveryScopeKind::Source,
                        value: "src".to_owned(),
                    };
                    MAX_DISCOVERY_FILTERS + 1
                ],
                ..base.clone()
            },
            DiscoveryQueryRequest {
                scope: vec![DiscoveryScope {
                    kind: DiscoveryScopeKind::Source,
                    value: String::new(),
                }],
                ..base.clone()
            },
            DiscoveryQueryRequest {
                scope: vec![DiscoveryScope {
                    kind: DiscoveryScopeKind::Source,
                    value: "x".repeat(MAX_DISCOVERY_FILTER_BYTES + 1),
                }],
                ..base
            },
        ];
        for request in invalid {
            let error = match super::validate_request(&request) {
                Ok(()) => std::process::abort(),
                Err(error) => error,
            };
            assert_eq!(error.kind(), crate::QueryErrorKind::InvalidParameter);
        }
    }

    #[test]
    fn edge_cap_has_exact_unique_omissions_and_coherent_endpoints()
    -> Result<(), Box<dyn std::error::Error>> {
        let engine = engine(
            vec![
                node("n:alpha", "alpha"),
                node("n:one", "one"),
                node("n:two", "two"),
            ],
            vec![
                edge("e:one", "n:alpha", "n:one"),
                edge("e:two", "n:alpha", "n:two"),
            ],
        );
        let mut bounded_request = request(DiscoveryDirection::Both);
        bounded_request.limits.max_edges = 1;
        let response = engine.discover(bounded_request)?;
        assert_eq!(response.edges.len(), 1);
        assert_eq!(response.omissions.edges, Some(1));
        let node_ids = response
            .nodes
            .iter()
            .map(|node| node.id.as_str())
            .collect::<std::collections::BTreeSet<_>>();
        assert!(response.edges.iter().all(|edge| {
            node_ids.contains(edge.source.as_str()) && node_ids.contains(edge.target.as_str())
        }));
        Ok(())
    }

    #[test]
    fn expansion_cap_bounds_high_degree_work_independent_of_graph_size()
    -> Result<(), Box<dyn std::error::Error>> {
        for degree in [32_usize, 1_024] {
            let mut nodes = vec![node("n:alpha", "alpha")];
            let mut edges = Vec::with_capacity(degree);
            for index in 0..degree {
                let id = format!("n:leaf:{index:04}");
                nodes.push(node(&id, &format!("leaf_{index:04}")));
                edges.push(edge(&format!("e:{index:04}"), "n:alpha", &id));
            }
            let engine = engine(nodes, edges);
            let mut bounded_request = request(DiscoveryDirection::Both);
            bounded_request.limits.max_expanded_relationships = 4;
            let response = engine.discover(bounded_request)?;
            assert!(response.stats.expanded_relationships <= 4);
            assert!(response.truncated);
            assert_eq!(response.omissions.expanded_relationships, None);
        }
        Ok(())
    }
}
