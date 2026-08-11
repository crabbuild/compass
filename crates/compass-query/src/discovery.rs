use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use compass_model::code_graph::{EdgeKind, EdgeRecord, NodeRecord};
use compass_model::provenance::EvidenceOrigin;
use compass_model::query_contract::{
    DISCOVERY_QUERY_SCHEMA_V1, DiscoveryAlternative, DiscoveryDirection, DiscoveryDirectionSource,
    DiscoveryEdge, DiscoveryLimits, DiscoveryOmissions, DiscoveryQueryRequest,
    DiscoveryQueryResponse, DiscoveryScope, DiscoveryScopeKind, DiscoveryScoreTier, DiscoverySeed,
    DiscoverySeedSource, DiscoveryStats, MAX_DISCOVERY_ALTERNATIVES_PER_SEED,
    MAX_DISCOVERY_CANDIDATE_NODES_READ, MAX_DISCOVERY_FILTER_BYTES, MAX_DISCOVERY_FILTERS,
    MAX_DISCOVERY_QUESTION_BYTES, QueryDiagnostic, QueryDiagnosticCode,
    canonical_discovery_scope_value, discovery_scope_matches,
};

use crate::code_query::{
    PinnedDiscoveryBackend, query_edge, query_node, recall_fuzzy_term_variants, search_query_terms,
};
use crate::ranking::{OperationRootRank, RelationEvidenceRank, rank_search_candidates};
use crate::recall::{
    CandidateSource, RecallBudget, RelationshipTermMatch, SearchCandidate, SearchCandidatePool,
};
use crate::text::{normalize_context_filters, search_tokens};
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

const MAX_SCOPE_AMBIGUITY_CANDIDATES: usize = 8;
const MAX_RELATIONSHIP_SUPPORT_TARGETS: usize = 8;
const MAX_DISCOVERY_INTERSECTION_PROBES: usize = 8;
const MAX_PRIMARY_INTERSECTION_ITEMS: usize = 2_048;

#[derive(Clone, Debug)]
struct RankedDiscoveryCandidate {
    node: NodeRecord,
    score: f64,
    channel_rank: u8,
    relation_evidence: Option<RelationEvidenceRank>,
    operation_root: Option<OperationRootRank>,
    matched_terms: Vec<String>,
    matched_fields: Vec<String>,
    source: DiscoverySeedSource,
}

struct DiscoveryCandidateSelection {
    candidates: Vec<RankedDiscoveryCandidate>,
    nodes_read: u64,
    probes: u64,
    expanded_relationships: u64,
    relationship_terms_supported: bool,
    ambiguity_complete: bool,
    truncated: bool,
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
        let backend = self.backend.pin_discovery()?;

        let relation_contexts = validate_and_normalize_contexts(&request.relation_contexts)?;
        let resolved_scope = self.resolve_scopes(&backend, &request.scope, &guard)?;
        let (selected_direction, direction_source) = match request.direction {
            DiscoveryDirection::Auto => infer_discovery_direction(&request.question),
            direction => (direction, DiscoveryDirectionSource::Explicit),
        };
        let mut response = DiscoveryQueryResponse {
            schema: DISCOVERY_QUERY_SCHEMA_V1.to_owned(),
            question: request.question.clone(),
            selected_direction,
            direction_source,
            relation_contexts,
            scope: resolved_scope,
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

        let selection = self.indexed_candidates(
            &backend,
            &request.question,
            &response.scope,
            selected_direction,
            &request.limits,
            &guard,
        )?;
        response.stats.candidate_nodes = selection.nodes_read;
        response.stats.candidate_probes = selection.probes;
        response.stats.expanded_relationships = selection.expanded_relationships;
        response.stats.candidates_admitted =
            u64::try_from(selection.candidates.len()).unwrap_or(u64::MAX);
        if selection.truncated {
            response.truncated = true;
        } else {
            response.omissions.candidates = Some(0);
        }
        if !selection.relationship_terms_supported {
            response.diagnostics.push(QueryDiagnostic {
                code: QueryDiagnosticCode::IncompleteCoverage,
                message: "The selected legacy graph snapshot lacks exact relationship-term postings; rebuild the graph for complete agent discovery recall".to_owned(),
                node_id: None,
                path: None,
            });
        }

        let max_seeds = usize::try_from(request.limits.max_seeds)
            .unwrap_or(usize::MAX)
            .min(usize::try_from(request.limits.max_nodes).unwrap_or(usize::MAX));
        let (seeds, omitted_alternatives) = discovery_seeds(&selection.candidates, max_seeds);
        response.seeds = seeds;
        if !selection.ambiguity_complete {
            for seed in &mut response.seeds {
                seed.ambiguous = true;
            }
        }
        response.omissions.alternatives = (!selection.truncated).then_some(omitted_alternatives);
        if omitted_alternatives > 0 {
            response.truncated = true;
            response.diagnostics.push(QueryDiagnostic {
                code: QueryDiagnosticCode::BoundedTruncation,
                message: format!(
                    "Ambiguity alternatives were limited; {omitted_alternatives} ranked alternative(s) were omitted"
                ),
                node_id: None,
                path: None,
            });
        }
        for seed in response.seeds.iter().filter(|seed| seed.ambiguous) {
            response.diagnostics.push(QueryDiagnostic {
                code: QueryDiagnosticCode::AmbiguousMatch,
                message: format!(
                    "Seed {} is ambiguous; retry with an exact node ID or run `compass explain {}`",
                    seed.node_id, seed.node_id
                ),
                node_id: Some(seed.node_id.clone()),
                path: None,
            });
        }
        if response.seeds.is_empty() {
            response.omissions.nodes = Some(0);
            response.omissions.edges = Some(0);
            response.omissions.expanded_relationships = Some(0);
            if response.truncated {
                response.diagnostics.push(QueryDiagnostic {
                    code: QueryDiagnosticCode::BoundedTruncation,
                    message: "Candidate recall was truncated before a scoped match could be proven; retry with an exact node ID or a narrower query".to_owned(),
                    node_id: None,
                    path: None,
                });
            } else {
                response.diagnostics.push(QueryDiagnostic {
                    code: QueryDiagnosticCode::NoMatch,
                    message: format!("No node matched {:?}", request.question),
                    node_id: None,
                    path: None,
                });
            };
            finish_response(&guard, &mut response)?;
            return Ok(response);
        }

        self.expand_reference_neighborhood(&backend, &request, &guard, &mut response)?;
        if !backend.supports_identifier_subwords()? {
            response.diagnostics.push(QueryDiagnostic {
                code: QueryDiagnosticCode::IncompleteCoverage,
                message: "The selected legacy graph snapshot lacks identifier-subword postings; rebuild the graph for complete agent discovery recall".to_owned(),
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
        finish_response(&guard, &mut response)?;
        Ok(response)
    }

    fn resolve_scopes(
        &self,
        backend: &PinnedDiscoveryBackend<'_>,
        requested: &[DiscoveryScope],
        guard: &DiscoveryGuard<'_>,
    ) -> Result<Vec<DiscoveryScope>, QueryError> {
        let mut resolved = Vec::with_capacity(requested.len());
        for scope in requested {
            guard.check()?;
            let Some(value) = canonical_discovery_scope_value(scope.kind, &scope.value) else {
                return Err(QueryError::new(
                    QueryErrorKind::InvalidParameter,
                    "invalid_discovery_scope",
                    format!("discovery scope {:?} has an invalid value", scope.kind),
                ));
            };
            let requested_scope = DiscoveryScope {
                kind: scope.kind,
                value,
            };
            let (values, truncated) = backend.resolve_scope_values(
                requested_scope.kind,
                &requested_scope.value,
                MAX_SCOPE_AMBIGUITY_CANDIDATES + 1,
            )?;
            if truncated {
                return Err(scope_resolution_error(
                    "ambiguous_discovery_scope",
                    &requested_scope,
                    &values,
                    true,
                ));
            }
            let canonical = canonical_resolved_scope(&requested_scope, &values)?;
            resolved.push(canonical);
        }
        Ok(canonical_scope(&resolved))
    }

    fn indexed_candidates(
        &self,
        backend: &PinnedDiscoveryBackend<'_>,
        question: &str,
        scope: &[DiscoveryScope],
        direction: DiscoveryDirection,
        limits: &DiscoveryLimits,
        guard: &DiscoveryGuard<'_>,
    ) -> Result<DiscoveryCandidateSelection, QueryError> {
        guard.check()?;
        let prepared = self.prepare_discovery_query(question)?;
        if prepared.fts_query.is_empty() {
            return Ok(DiscoveryCandidateSelection {
                candidates: Vec::new(),
                nodes_read: 0,
                probes: 0,
                expanded_relationships: 0,
                relationship_terms_supported: backend.supports_relationship_terms()?,
                ambiguity_complete: true,
                truncated: false,
            });
        }
        let candidate_limit = usize::try_from(limits.max_candidates).unwrap_or(usize::MAX);
        let promotion_limit = (candidate_limit / 2).min(128);
        let direct_limit = candidate_limit.saturating_sub(promotion_limit).min(128);
        let candidate_read_limit =
            usize::try_from(MAX_DISCOVERY_CANDIDATE_NODES_READ).unwrap_or(usize::MAX);
        let candidate_probe_limit =
            usize::try_from(compass_model::query_contract::MAX_DISCOVERY_CANDIDATE_PROBES)
                .unwrap_or(usize::MAX);
        let mut concepts = prepared
            .ranking_terms
            .iter()
            .cloned()
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect::<Vec<_>>();
        concepts.sort_by(|left, right| {
            crate::ranking::canonical_predicate_token(right)
                .is_some()
                .cmp(&crate::ranking::canonical_predicate_token(left).is_some())
                .then_with(|| right.len().cmp(&left.len()))
                .then_with(|| left.cmp(right))
        });
        let mut pool = SearchCandidatePool::new(RecallBudget {
            max_total_candidates: direct_limit,
            max_per_source: direct_limit,
            max_fuzzy_candidates: direct_limit.min(16),
        });
        let mut nodes_read = 0_usize;
        let mut probes = 0_usize;
        let mut truncated = false;
        let mut term_postings_truncated = false;
        let mut exact_name_recall_complete = false;
        let mut complete_direct_terms = BTreeSet::new();

        guard.check()?;
        if probes < candidate_probe_limit && nodes_read < candidate_read_limit {
            probes += 1;
            if let Some(node) = backend.node_by_id(question)? {
                nodes_read += 1;
                if discovery_scope_matches(&node, scope) {
                    let id = node.id.clone();
                    let _ = pool.add(CandidateSource::ExactId, node);
                    let _ = pool.add_indexed_matches(&id, concepts.clone());
                }
            }
        }

        guard.check()?;
        if probes < candidate_probe_limit && nodes_read < candidate_read_limit {
            probes += 1;
            let remaining = candidate_read_limit.saturating_sub(nodes_read).min(64);
            let (nodes, read_truncated) =
                backend.nodes_by_normalized_name(question, remaining.max(1))?;
            exact_name_recall_complete = !read_truncated;
            nodes_read = nodes_read.saturating_add(nodes.len());
            truncated |= read_truncated;
            for node in nodes {
                if discovery_scope_matches(&node, scope) {
                    let id = node.id.clone();
                    let _ = pool.add(CandidateSource::ExactName, node);
                    let _ = pool.add_indexed_matches(&id, concepts.clone());
                }
            }
        }

        // An absent composite identifier is intentionally exact-only: the
        // final specificity gate below rejects every alias, term,
        // relationship, and fuzzy candidate for this query shape. Finish a
        // proven no-match now instead of hydrating generic postings that
        // cannot be admitted. Exact hits continue through the normal ranking
        // path so their contextual seeds and ambiguity remain unchanged.
        if is_composite_identifier_query(question, &prepared.ranking_terms)
            && pool.len() == 0
            && exact_name_recall_complete
        {
            return Ok(DiscoveryCandidateSelection {
                candidates: Vec::new(),
                nodes_read: u64::try_from(nodes_read).unwrap_or(u64::MAX),
                probes: u64::try_from(probes).unwrap_or(u64::MAX),
                expanded_relationships: 0,
                relationship_terms_supported: backend.supports_relationship_terms()?,
                ambiguity_complete: true,
                truncated: false,
            });
        }

        let reserved_term_capacity = direct_limit / 2;
        let non_term_capacity = direct_limit.saturating_sub(reserved_term_capacity);
        for concept in &concepts {
            guard.check()?;
            if probes >= candidate_probe_limit || nodes_read >= candidate_read_limit {
                truncated = true;
                break;
            }
            probes += 1;
            let alias_limit = candidate_read_limit.saturating_sub(nodes_read).min(128);
            let (nodes, alias_truncated) =
                backend.nodes_by_normalized_name(concept, alias_limit.max(1))?;
            nodes_read = nodes_read.saturating_add(nodes.len());
            truncated |= alias_truncated;
            for node in nodes {
                if pool.len() >= non_term_capacity {
                    break;
                }
                if discovery_scope_matches(&node, scope) {
                    let id = node.id.clone();
                    let _ = pool.add(CandidateSource::Alias, node);
                    let _ = pool.add_indexed_matches(&id, [concept.clone()]);
                }
            }
        }

        let mut term_candidates = BTreeMap::<String, SearchCandidate>::new();
        let lexical_read_limit = if concepts.len() >= 2 && direction != DiscoveryDirection::Incoming
        {
            candidate_read_limit / 2
        } else {
            candidate_read_limit
        };
        let intersection_terms = selective_intersection_terms(&concepts);
        for (intersection_index, intersection) in intersection_terms.iter().enumerate() {
            guard.check()?;
            let remaining = lexical_read_limit.saturating_sub(nodes_read);
            let groups_remaining = intersection_terms
                .len()
                .saturating_sub(intersection_index)
                .saturating_add(concepts.len())
                .max(1);
            let fair_limit = remaining / groups_remaining;
            let fair_limit = if intersection_index == 0 {
                fair_limit.max(remaining.min(MAX_PRIMARY_INTERSECTION_ITEMS))
            } else {
                fair_limit
            };
            let minimum =
                compass_graph::GRAPH_TERM_POSTING_CHUNK_ITEMS.saturating_mul(intersection.len());
            if probes >= candidate_probe_limit || fair_limit < minimum {
                term_postings_truncated = true;
                break;
            }
            probes += 1;
            let read = self.discovery_term_candidates(backend, intersection, fair_limit)?;
            nodes_read = nodes_read
                .saturating_add(usize::try_from(read.node_ids_decoded).unwrap_or(usize::MAX));
            probes =
                probes.saturating_add(usize::try_from(read.chunks_decoded).unwrap_or(usize::MAX));
            term_postings_truncated |=
                read.truncated || nodes_read > lexical_read_limit || probes > candidate_probe_limit;
            for node in read.nodes {
                if !discovery_scope_matches(&node, scope) {
                    continue;
                }
                let matched = read
                    .matched_concepts
                    .get(&node.id)
                    .cloned()
                    .unwrap_or_default();
                let candidate =
                    term_candidates
                        .entry(node.id.clone())
                        .or_insert_with(|| SearchCandidate {
                            node,
                            sources: BTreeSet::from([CandidateSource::TermIndex]),
                            indexed_matches: BTreeSet::new(),
                            relationship_matches: BTreeSet::new(),
                        });
                candidate.indexed_matches.extend(matched);
            }
        }
        for (concept_index, concept) in concepts.iter().enumerate() {
            guard.check()?;
            let remaining = lexical_read_limit.saturating_sub(nodes_read);
            let concepts_remaining = concepts.len().saturating_sub(concept_index).max(1);
            let fair_limit = remaining / concepts_remaining;
            if probes >= candidate_probe_limit
                || fair_limit < compass_graph::GRAPH_TERM_POSTING_CHUNK_ITEMS
            {
                term_postings_truncated = true;
                break;
            }
            probes += 1;
            let read =
                self.discovery_term_candidates(backend, std::slice::from_ref(concept), fair_limit)?;
            if !read.truncated {
                complete_direct_terms.insert(concept.clone());
            }
            nodes_read = nodes_read
                .saturating_add(usize::try_from(read.node_ids_decoded).unwrap_or(usize::MAX));
            probes =
                probes.saturating_add(usize::try_from(read.chunks_decoded).unwrap_or(usize::MAX));
            term_postings_truncated |=
                read.truncated || nodes_read > lexical_read_limit || probes > candidate_probe_limit;
            for node in read.nodes {
                if !discovery_scope_matches(&node, scope)
                    || !read
                        .matched_concepts
                        .get(&node.id)
                        .is_some_and(|matched| matched.contains(concept))
                {
                    continue;
                }
                let candidate =
                    term_candidates
                        .entry(node.id.clone())
                        .or_insert_with(|| SearchCandidate {
                            node,
                            sources: BTreeSet::from([CandidateSource::TermIndex]),
                            indexed_matches: BTreeSet::new(),
                            relationship_matches: BTreeSet::new(),
                        });
                candidate.indexed_matches.insert(concept.clone());
            }
        }

        let direct_available = direct_limit.saturating_sub(pool.len());
        let ranked_direct = rank_search_candidates(
            question,
            &prepared.ranking_terms,
            term_candidates.into_values().collect(),
            direct_available,
        );
        for candidate in ranked_direct {
            let id = candidate.node.id.clone();
            let _ = pool.add(CandidateSource::TermIndex, candidate.node);
            let _ = pool.add_indexed_matches(&id, candidate.matched_terms);
        }

        let mut expanded_relationships = 0_u64;
        let mut complete_relationship_candidate = false;
        let relationship_terms_supported = backend.supports_relationship_terms()?;
        truncated |= !relationship_terms_supported;
        if relationship_terms_supported
            && concepts.len() >= 2
            && promotion_limit > 0
            && direction != DiscoveryDirection::Incoming
        {
            let recall_edge_limit = limits.max_expanded_relationships.min(4_096);
            let posting_budget = recall_edge_limit / 2;
            let mut complete_masks = BTreeMap::<String, BTreeSet<String>>::new();
            let mut observed_truncated_postings =
                BTreeMap::<String, (BTreeSet<String>, Option<String>)>::new();
            let mut exhaustive_postings = 0_usize;
            let mut truncated_concepts = Vec::<String>::new();
            let mut concepts_examined = 0_usize;
            for (concept_index, concept) in concepts.iter().enumerate() {
                guard.check()?;
                let concepts_remaining = concepts.len().saturating_sub(concept_index).max(1);
                let remaining_nodes = candidate_read_limit.saturating_sub(nodes_read);
                let remaining_relationships =
                    usize::try_from(posting_budget.saturating_sub(expanded_relationships))
                        .unwrap_or(usize::MAX);
                let fair_limit = (remaining_nodes / concepts_remaining)
                    .min(remaining_relationships / concepts_remaining);
                if probes >= candidate_probe_limit
                    || fair_limit < compass_graph::GRAPH_TERM_POSTING_CHUNK_ITEMS
                {
                    truncated = true;
                    break;
                }
                probes += 1;
                let read = self.discovery_relationship_sources(backend, concept, fair_limit)?;
                concepts_examined = concepts_examined.saturating_add(1);
                nodes_read = nodes_read
                    .saturating_add(usize::try_from(read.node_ids_decoded).unwrap_or(usize::MAX));
                probes = probes
                    .saturating_add(usize::try_from(read.chunks_decoded).unwrap_or(usize::MAX));
                expanded_relationships =
                    expanded_relationships.saturating_add(read.node_ids_decoded);
                if read.truncated {
                    truncated_concepts.push(concept.clone());
                    let complete_through_source_id = read.source_ids.last().cloned();
                    observed_truncated_postings.insert(
                        concept.clone(),
                        (
                            read.source_ids.into_iter().collect(),
                            complete_through_source_id,
                        ),
                    );
                } else {
                    exhaustive_postings += 1;
                    for source_id in read.source_ids {
                        complete_masks
                            .entry(source_id)
                            .or_default()
                            .insert(concept.clone());
                    }
                }
            }
            let all_concepts_examined = concepts_examined == concepts.len();
            if !all_concepts_examined || exhaustive_postings == 0 || truncated_concepts.len() > 1 {
                truncated = true;
            }
            let mut relationship_proof_complete =
                all_concepts_examined && exhaustive_postings > 0 && truncated_concepts.len() <= 1;
            let mut relationship_masks = BTreeMap::<String, BTreeSet<String>>::new();
            for (source_id, mut matches) in complete_masks {
                guard.check()?;
                let mut verified = true;
                for concept in &truncated_concepts {
                    if let Some((source_ids, complete_through_source_id)) =
                        observed_truncated_postings.get(concept)
                    {
                        if source_ids.contains(&source_id) {
                            matches.insert(concept.clone());
                            continue;
                        }
                        if complete_through_source_id
                            .as_ref()
                            .is_some_and(|last| source_id.as_str() <= last.as_str())
                        {
                            continue;
                        }
                    }
                    if probes >= candidate_probe_limit
                        || expanded_relationships >= recall_edge_limit
                    {
                        truncated = true;
                        relationship_proof_complete = false;
                        verified = false;
                        break;
                    }
                    probes += 1;
                    expanded_relationships = expanded_relationships.saturating_add(1);
                    if self
                        .discovery_relationship_source_matches_term(backend, &source_id, concept)?
                    {
                        matches.insert(concept.clone());
                    }
                }
                if !verified {
                    break;
                }
                if matches.len() >= 2 {
                    relationship_masks.insert(source_id, matches);
                }
            }
            let mut hydrated_promotions = Vec::new();
            for (source_id, matches) in relationship_masks {
                guard.check()?;
                if probes >= candidate_probe_limit || nodes_read >= candidate_read_limit {
                    truncated = true;
                    relationship_proof_complete = false;
                    break;
                }
                probes += 1;
                let Some(node) = backend.node_by_id(&source_id)? else {
                    return Err(QueryError::new(
                        QueryErrorKind::GraphInvariant,
                        "discovery_relationship_source_missing",
                        format!("relationship-term index references absent node {source_id}"),
                    ));
                };
                nodes_read += 1;
                if node.kind.is_callable()
                    && node.source_file().is_some_and(|file| !file.is_empty())
                    && discovery_scope_matches(&node, scope)
                {
                    if probes >= candidate_probe_limit
                        || expanded_relationships >= recall_edge_limit
                    {
                        truncated = true;
                        relationship_proof_complete = false;
                        break;
                    }
                    let remaining =
                        usize::try_from(recall_edge_limit.saturating_sub(expanded_relationships))
                            .unwrap_or(usize::MAX);
                    let target_limit = MAX_RELATIONSHIP_SUPPORT_TARGETS
                        .saturating_mul(matches.len())
                        .saturating_add(1)
                        .min(remaining);
                    if target_limit == 0 {
                        truncated = true;
                        relationship_proof_complete = false;
                        break;
                    }
                    probes += 1;
                    let read = self.discovery_relationship_targets(
                        backend,
                        &source_id,
                        &matches,
                        target_limit,
                    )?;
                    expanded_relationships =
                        expanded_relationships.saturating_add(read.ids_decoded);
                    let retained_targets = read
                        .target_ids
                        .into_iter()
                        .take(MAX_RELATIONSHIP_SUPPORT_TARGETS)
                        .collect::<BTreeSet<_>>();
                    if read.truncated && retained_targets.len() < MAX_RELATIONSHIP_SUPPORT_TARGETS {
                        truncated = true;
                        relationship_proof_complete = false;
                    }
                    let relationship_matches = matches
                        .into_iter()
                        .map(|term| RelationshipTermMatch {
                            target_ids: retained_targets.clone(),
                            term,
                            kind: EdgeKind::Calls,
                        })
                        .collect::<BTreeSet<_>>();
                    hydrated_promotions.push((node, relationship_matches));
                }
            }
            pool.extend_total_budget(candidate_limit);
            let promotion_count = hydrated_promotions.len();
            let mut promotion_matches = BTreeMap::new();
            let promotion_candidates = hydrated_promotions
                .into_iter()
                .map(|(node, relationship_matches)| {
                    promotion_matches.insert(node.id.clone(), relationship_matches.clone());
                    SearchCandidate {
                        node,
                        sources: BTreeSet::from([CandidateSource::RelationSeed]),
                        indexed_matches: BTreeSet::new(),
                        relationship_matches,
                    }
                })
                .collect::<Vec<_>>();
            if promotion_count > promotion_limit {
                truncated = true;
                relationship_proof_complete = false;
            }
            let promotions = rank_search_candidates(
                question,
                &prepared.ranking_terms,
                promotion_candidates,
                promotion_limit,
            );
            let mut promoted_any = false;
            for promotion in promotions {
                let node = promotion.node;
                let id = node.id.clone();
                let inserted = pool.add(CandidateSource::RelationSeed, node);
                let matches = promotion_matches.remove(&id).unwrap_or_default();
                // A pre-existing direct candidate still counts after the
                // exact relationship evidence is attached. A capacity-
                // rejected node does not.
                let relationship_attached = pool.add_relationship_matches(&id, matches);
                promoted_any |= inserted || relationship_attached;
            }
            complete_relationship_candidate = relationship_proof_complete && promoted_any;
        } else {
            pool.extend_total_budget(candidate_limit);
        }

        if !complete_relationship_candidate && pool.len() < candidate_limit.min(4) {
            let mut fuzzy_remaining = 16_usize.min(candidate_limit.saturating_sub(pool.len()));
            let mut fuzzy_probes_remaining = 16_usize;
            for variant in recall_fuzzy_term_variants(&prepared.terms) {
                if fuzzy_remaining == 0 || fuzzy_probes_remaining == 0 {
                    break;
                }
                if probes >= candidate_probe_limit || nodes_read >= candidate_read_limit {
                    truncated = true;
                    break;
                }
                probes += 1;
                fuzzy_probes_remaining = fuzzy_probes_remaining.saturating_sub(1);
                let (nodes, fuzzy_truncated) =
                    backend.nodes_by_normalized_name(&variant, fuzzy_remaining)?;
                nodes_read = nodes_read.saturating_add(nodes.len());
                truncated |= fuzzy_truncated;
                for node in nodes {
                    if discovery_scope_matches(&node, scope)
                        && pool.add(CandidateSource::Fuzzy, node)
                    {
                        fuzzy_remaining = fuzzy_remaining.saturating_sub(1);
                    }
                }
            }
        }
        truncated |= term_postings_truncated && !complete_relationship_candidate;
        truncated |= pool.is_truncated();
        let mut ranked = rank_search_candidates(
            question,
            &prepared.ranking_terms,
            pool.into_vec(),
            candidate_limit,
        );
        let specificity_no_match =
            retain_specific_discovery_candidates(question, &prepared.ranking_terms, &mut ranked);
        let no_match_complete = specificity_no_match && exact_name_recall_complete;
        let effective_truncated = truncated && !no_match_complete;
        let exact_dominance_complete = ranked.first().is_some_and(|candidate| {
            candidate.channel_rank == 6
                || (candidate.channel_rank == 5 && exact_name_recall_complete)
        });
        let operation_dominance_complete = ranked.first().is_some_and(|candidate| {
            candidate.operation_root.is_some()
                && operation_predicate_posting_complete(
                    &candidate.node,
                    &prepared.ranking_terms,
                    &complete_direct_terms,
                )
                && ranked
                    .get(1)
                    .is_none_or(|runner_up| candidate.operation_root > runner_up.operation_root)
        });
        let ambiguity_complete = !effective_truncated
            || exact_dominance_complete
            || operation_dominance_complete
            || (complete_relationship_candidate
                && ranked
                    .first()
                    .is_some_and(|candidate| candidate.channel_rank == 4));
        guard.check()?;
        let mut candidates = Vec::with_capacity(ranked.len());
        for result in ranked {
            guard.check()?;
            candidates.push(RankedDiscoveryCandidate {
                matched_terms: result.matched_terms,
                matched_fields: result.matched_fields,
                source: discovery_candidate_source(result.candidate_source),
                node: result.node,
                score: result.score,
                channel_rank: result.channel_rank,
                relation_evidence: result.relation_evidence,
                operation_root: result.operation_root,
            });
        }
        Ok(DiscoveryCandidateSelection {
            candidates,
            nodes_read: u64::try_from(nodes_read).unwrap_or(u64::MAX),
            probes: u64::try_from(probes).unwrap_or(u64::MAX),
            expanded_relationships,
            relationship_terms_supported,
            ambiguity_complete,
            truncated: effective_truncated,
        })
    }

    fn expand_reference_neighborhood(
        &self,
        backend: &PinnedDiscoveryBackend<'_>,
        request: &DiscoveryQueryRequest,
        guard: &DiscoveryGuard<'_>,
        response: &mut DiscoveryQueryResponse,
    ) -> Result<(), QueryError> {
        let max_nodes = usize::try_from(request.limits.max_nodes).unwrap_or(usize::MAX);
        let max_expanded = request.limits.max_expanded_relationships;
        let max_depth = usize::try_from(request.limits.max_depth).unwrap_or(usize::MAX);
        let mut selected_nodes = BTreeMap::<String, NodeRecord>::new();
        let mut omitted_node_ids = BTreeSet::new();
        let mut visited = BTreeSet::new();
        let mut frontier = VecDeque::new();
        let mut membership_complete = true;
        let mut edge_cache = BTreeMap::new();

        for seed in &response.seeds {
            guard.check()?;
            let Some(node) = backend.node_by_id(&seed.node_id)? else {
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

        'traversal: while let Some((node_id, depth)) = match request.traversal {
            compass_model::query_contract::DiscoveryTraversal::Bfs => frontier.pop_front(),
            compass_model::query_contract::DiscoveryTraversal::Dfs => frontier.pop_back(),
        } {
            guard.check()?;
            if depth >= max_depth || response.stats.expanded_relationships >= max_expanded {
                if response.stats.expanded_relationships >= max_expanded {
                    mark_expansion_truncated(response);
                    membership_complete = false;
                }
                continue;
            }
            let remaining_expansion =
                usize::try_from(max_expanded.saturating_sub(response.stats.expanded_relationships))
                    .unwrap_or(usize::MAX);
            // Once every remaining node slot plus one witness has been
            // examined, more adjacency cannot change the bounded node set.
            // Reading a larger Store prefix only hydrates edges that the
            // response cannot admit. A truncated prefix keeps completeness
            // and omission counts honest.
            let remaining_node_slots = max_nodes.saturating_sub(selected_nodes.len());
            let adjacency_limit = remaining_expansion
                .min(remaining_node_slots.saturating_add(1))
                .max(1);
            let (edges, truncated) = edges_for_direction(
                backend,
                &node_id,
                response.selected_direction,
                request.include_heuristic,
                adjacency_limit,
            )?;
            if truncated {
                mark_expansion_truncated(response);
                membership_complete = false;
            }
            for edge in edges {
                guard.check()?;
                if !edge.id.is_empty() {
                    edge_cache
                        .entry(edge.id.clone())
                        .or_insert_with(|| edge.clone());
                }
                response.stats.expanded_relationships =
                    response.stats.expanded_relationships.saturating_add(1);
                if response.stats.expanded_relationships > max_expanded {
                    mark_expansion_truncated(response);
                    membership_complete = false;
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
                if selected_nodes.contains_key(&other_id) {
                    if visited.insert(other_id.clone()) {
                        frontier.push_back((other_id, depth.saturating_add(1)));
                    }
                    continue;
                }
                if selected_nodes.len() >= max_nodes {
                    response.truncated = true;
                    membership_complete = false;
                    omitted_node_ids.insert(other_id);
                    break 'traversal;
                }
                let Some(other) = backend.node_by_id(&other_id)? else {
                    return Err(QueryError::new(
                        QueryErrorKind::GraphInvariant,
                        "discovery_edge_endpoint_missing",
                        format!("edge {} references absent node {other_id}", edge.id),
                    ));
                };
                if !discovery_scope_matches(&other, &response.scope) {
                    continue;
                }
                selected_nodes.entry(other.id.clone()).or_insert(other);
                if visited.insert(other_id.clone()) {
                    frontier.push_back((other_id, depth.saturating_add(1)));
                }
            }
        }

        response.nodes = selected_nodes.values().map(query_node).collect();
        response.omissions.nodes =
            membership_complete.then(|| u64::try_from(omitted_node_ids.len()).unwrap_or(u64::MAX));
        let edge_assembly_complete = self.assemble_selected_edges(
            backend,
            request,
            guard,
            &selected_nodes,
            &edge_cache,
            response,
        )?;
        response.omissions.expanded_relationships =
            (membership_complete && edge_assembly_complete).then_some(0);
        response.stats.visited_nodes = u64::try_from(visited.len()).unwrap_or(u64::MAX);
        Ok(())
    }

    fn assemble_selected_edges(
        &self,
        backend: &PinnedDiscoveryBackend<'_>,
        request: &DiscoveryQueryRequest,
        guard: &DiscoveryGuard<'_>,
        selected_nodes: &BTreeMap<String, NodeRecord>,
        cached_edges: &BTreeMap<String, EdgeRecord>,
        response: &mut DiscoveryQueryResponse,
    ) -> Result<bool, QueryError> {
        let max_edges = usize::try_from(request.limits.max_edges).unwrap_or(usize::MAX);
        let max_expanded = request.limits.max_expanded_relationships;
        let mut selected_edges = Vec::<(usize, EdgeRecord)>::new();
        let mut selected_store_edge_ids = BTreeSet::new();
        let mut complete = true;
        let selected_node_ids = selected_nodes.keys().cloned().collect::<BTreeSet<_>>();
        for node_id in selected_nodes.keys() {
            guard.check()?;
            let remaining =
                usize::try_from(max_expanded.saturating_sub(response.stats.expanded_relationships))
                    .unwrap_or(usize::MAX);
            if remaining == 0 {
                complete = false;
                mark_expansion_truncated(response);
                break;
            }
            let read = backend.outgoing_within_nodes_bounded_work(
                node_id,
                &selected_node_ids,
                request.include_heuristic,
                remaining,
            )?;
            response.stats.expanded_relationships = response
                .stats
                .expanded_relationships
                .saturating_add(u64::try_from(read.examined).unwrap_or(u64::MAX));
            if read.truncated {
                complete = false;
                mark_expansion_truncated(response);
            }
            for edge in read.records {
                guard.check()?;
                if !edge_matches_context(&edge, &response.relation_contexts) {
                    continue;
                }
                // Each stored outgoing occurrence is visited exactly once for
                // its authoritative source node. The encounter ordinal is a
                // final tie-break only; optional public IDs are never used as
                // multigraph deduplication keys.
                selected_edges.push((selected_edges.len(), edge));
            }
            selected_store_edge_ids.extend(read.edge_ids);
            if read.truncated {
                break;
            }
        }
        let uncached_edge_ids = selected_store_edge_ids
            .iter()
            .filter(|edge_id| !cached_edges.contains_key(*edge_id))
            .cloned()
            .collect::<BTreeSet<_>>();
        let mut loaded_edges = backend
            .edges_by_ids(&uncached_edge_ids)?
            .into_iter()
            .map(|edge| (edge.id.clone(), edge))
            .collect::<BTreeMap<_, _>>();
        for edge_id in selected_store_edge_ids {
            guard.check()?;
            let edge = if let Some(edge) = cached_edges.get(&edge_id) {
                edge.clone()
            } else {
                loaded_edges.remove(&edge_id).ok_or_else(|| {
                    QueryError::new(
                        QueryErrorKind::GraphInvariant,
                        "discovery_edge_missing",
                        format!("outgoing index references missing edge {edge_id}"),
                    )
                })?
            };
            if !edge_matches_context(&edge, &response.relation_contexts)
                || (!request.include_heuristic && is_heuristic(&edge))
            {
                continue;
            }
            selected_edges.push((selected_edges.len(), edge));
        }
        selected_edges.sort_by(compare_discovery_edge_occurrences);
        let omitted_edges = selected_edges.len().saturating_sub(max_edges);
        if omitted_edges > 0 {
            response.truncated = true;
            selected_edges.truncate(max_edges);
        }
        response.edges = selected_edges
            .iter()
            .map(|(_, edge)| discovery_edge(edge))
            .collect();
        response.omissions.edges =
            complete.then(|| u64::try_from(omitted_edges).unwrap_or(u64::MAX));
        Ok(complete)
    }
}

fn retain_specific_discovery_candidates(
    question: &str,
    terms: &[String],
    ranked: &mut Vec<crate::ranking::RankedSearchResult>,
) -> bool {
    let distinct_terms = terms.iter().collect::<BTreeSet<_>>().len();
    if is_composite_identifier_query(question, terms) {
        if !ranked.iter().any(|candidate| candidate.channel_rank >= 5) {
            ranked.clear();
            return true;
        }
        return false;
    }
    if distinct_terms < 3
        || ranked.iter().any(|candidate| {
            candidate.channel_rank >= 4
                || candidate.matched_terms.len() >= 2
                || candidate.relation_evidence.is_some()
        })
    {
        return false;
    }
    ranked.clear();
    false
}

fn is_composite_identifier_query(question: &str, terms: &[String]) -> bool {
    !question.chars().any(char::is_whitespace)
        && question
            .chars()
            .all(|character| character.is_alphanumeric() || character == '_')
        && terms.iter().collect::<BTreeSet<_>>().len() >= 3
}

fn operation_predicate_posting_complete(
    node: &NodeRecord,
    terms: &[String],
    complete_direct_terms: &BTreeSet<String>,
) -> bool {
    search_tokens(&node.name).into_iter().any(|term| {
        crate::ranking::canonical_predicate_token(&term).is_some()
            && terms.binary_search(&term).is_ok()
            && complete_direct_terms.contains(&term)
    })
}

fn selective_intersection_terms(concepts: &[String]) -> Vec<Vec<String>> {
    if concepts.len() < 2 {
        return Vec::new();
    }
    let mut pairs = Vec::new();
    for left in 0..concepts.len() {
        for right in left.saturating_add(1)..concepts.len() {
            pairs.push(vec![concepts[left].clone(), concepts[right].clone()]);
        }
    }
    pairs.sort_by(|left, right| {
        intersection_predicate_count(right)
            .cmp(&intersection_predicate_count(left))
            .then_with(|| intersection_token_bytes(right).cmp(&intersection_token_bytes(left)))
            .then_with(|| left.cmp(right))
    });

    let mut intersections = pairs;
    if concepts.len() <= 4 {
        intersections.push(concepts.to_vec());
    }
    intersections.truncate(MAX_DISCOVERY_INTERSECTION_PROBES);
    intersections
}

fn intersection_predicate_count(concepts: &[String]) -> usize {
    concepts
        .iter()
        .filter(|concept| crate::ranking::canonical_predicate_token(concept).is_some())
        .count()
}

fn intersection_token_bytes(concepts: &[String]) -> usize {
    concepts.iter().map(String::len).sum()
}

fn infer_discovery_direction(question: &str) -> (DiscoveryDirection, DiscoveryDirectionSource) {
    let normalized = question
        .chars()
        .map(|character| {
            if character.is_alphanumeric() {
                character.to_ascii_lowercase()
            } else {
                ' '
            }
        })
        .collect::<String>();
    let normalized = normalized.split_whitespace().collect::<Vec<_>>().join(" ");
    let incoming = [
        "caller",
        "called by",
        "used by",
        "registered by",
        "enforced at",
        "affected by",
        "referenced by",
        "depends on this",
    ]
    .iter()
    .any(|signal| normalized.contains(signal));
    let outgoing = [
        " calls ",
        " invokes ",
        " uses ",
        "implementation flow",
        "writes to",
        "calls from",
    ]
    .iter()
    .any(|signal| format!(" {normalized} ").contains(signal))
        || (normalized.contains("depends on") && !normalized.contains("depends on this"));
    if incoming && outgoing {
        return (DiscoveryDirection::Both, DiscoveryDirectionSource::Neutral);
    }
    if let Ok(plan) = crate::intent::plan_natural_query(question)
        && plan.routes_to_typed_query()
    {
        match plan.intent() {
            crate::intent::NaturalQueryIntent::Callers
            | crate::intent::NaturalQueryIntent::Impact => {
                return (
                    DiscoveryDirection::Incoming,
                    DiscoveryDirectionSource::Heuristic,
                );
            }
            crate::intent::NaturalQueryIntent::Callees => {
                return (
                    DiscoveryDirection::Outgoing,
                    DiscoveryDirectionSource::Heuristic,
                );
            }
            crate::intent::NaturalQueryIntent::NodeTrail => {
                return (
                    DiscoveryDirection::Outgoing,
                    DiscoveryDirectionSource::Heuristic,
                );
            }
            crate::intent::NaturalQueryIntent::Search
            | crate::intent::NaturalQueryIntent::Fallback => {}
        }
    }
    let neutral = ["architecture", "related", "connected", "coupling"]
        .iter()
        .any(|signal| normalized.contains(signal))
        || (normalized.contains("flow") && !normalized.contains("implementation flow"));
    if neutral {
        return (DiscoveryDirection::Both, DiscoveryDirectionSource::Neutral);
    }
    let outgoing = outgoing || normalized.starts_with("how does ");
    match (incoming, outgoing) {
        (true, false) => (
            DiscoveryDirection::Incoming,
            DiscoveryDirectionSource::Heuristic,
        ),
        (false, true) => (
            DiscoveryDirection::Outgoing,
            DiscoveryDirectionSource::Heuristic,
        ),
        _ => (DiscoveryDirection::Both, DiscoveryDirectionSource::Neutral),
    }
}

fn validate_and_normalize_contexts(filters: &[String]) -> Result<Vec<String>, QueryError> {
    const SUPPORTED: &[&str] = &[
        "attribute",
        "call",
        "declaration",
        "dependency",
        "export",
        "field",
        "generic_arg",
        "import",
        "parameter_type",
        "read",
        "registration",
        "return_type",
        "route",
        "test",
        "type",
        "write",
    ];
    let normalized = normalize_context_filters(filters);
    if let Some(value) = normalized
        .iter()
        .find(|value| !SUPPORTED.contains(&value.as_str()))
    {
        return Err(QueryError::new(
            QueryErrorKind::InvalidParameter,
            "unsupported_discovery_context",
            format!(
                "unsupported relationship context {value:?}; supported canonical contexts are {}",
                SUPPORTED.join(", ")
            ),
        ));
    }
    Ok(normalized)
}

fn canonical_resolved_scope(
    requested: &DiscoveryScope,
    values: &[String],
) -> Result<DiscoveryScope, QueryError> {
    if values.is_empty() {
        return Err(scope_resolution_error(
            "unknown_discovery_scope",
            requested,
            values,
            false,
        ));
    }
    match requested.kind {
        DiscoveryScopeKind::Source | DiscoveryScopeKind::Package => Ok(requested.clone()),
        DiscoveryScopeKind::Node | DiscoveryScopeKind::Community => {
            let canonical = if values.iter().any(|value| value == &requested.value) {
                requested.value.clone()
            } else if values.len() == 1 {
                values[0].clone()
            } else {
                return Err(scope_resolution_error(
                    "ambiguous_discovery_scope",
                    requested,
                    values,
                    false,
                ));
            };
            Ok(DiscoveryScope {
                kind: requested.kind,
                value: canonical,
            })
        }
    }
}

fn scope_resolution_error(
    code: &'static str,
    scope: &DiscoveryScope,
    values: &[String],
    truncated: bool,
) -> QueryError {
    let identity_scope = matches!(
        scope.kind,
        DiscoveryScopeKind::Community | DiscoveryScopeKind::Node
    );
    let candidates = values
        .iter()
        .take(MAX_SCOPE_AMBIGUITY_CANDIDATES)
        .map(String::as_str)
        .collect::<Vec<_>>()
        .join(", ");
    let candidate_label = if identity_scope {
        "candidate IDs"
    } else {
        "matching values"
    };
    let detail = if candidates.is_empty() {
        String::new()
    } else if truncated {
        format!("; {candidate_label} include {candidates}, with more omitted")
    } else {
        format!("; {candidate_label}: {candidates}")
    };
    let guidance = match scope.kind {
        DiscoveryScopeKind::Source => "use an existing normalized source path or path prefix",
        DiscoveryScopeKind::Package => "use an existing normalized package name or package prefix",
        DiscoveryScopeKind::Community => "use an exact community ID",
        DiscoveryScopeKind::Node => "use an exact node ID",
    };
    QueryError::new(
        QueryErrorKind::InvalidParameter,
        code,
        format!(
            "scope {:?}={:?} is not uniquely resolvable{detail}; {guidance}",
            scope.kind, scope.value,
        ),
    )
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
    let _ = search_query_terms(&request.question)?;
    validate_filters("relationContexts", &request.relation_contexts)?;
    if request.scope.len() > MAX_DISCOVERY_FILTERS
        || request.scope.iter().any(|scope| {
            scope.value.trim().is_empty() || scope.value.len() > MAX_DISCOVERY_FILTER_BYTES
        })
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
            .any(|value| value.trim().is_empty() || value.len() > MAX_DISCOVERY_FILTER_BYTES)
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

fn discovery_candidate_source(source: CandidateSource) -> DiscoverySeedSource {
    match source {
        CandidateSource::ExactId => DiscoverySeedSource::ExactId,
        CandidateSource::ExactName => DiscoverySeedSource::ExactName,
        CandidateSource::Alias => DiscoverySeedSource::Alias,
        CandidateSource::TermIndex => DiscoverySeedSource::TermIndex,
        CandidateSource::RelationSeed => DiscoverySeedSource::RelationSeed,
        CandidateSource::Fuzzy => DiscoverySeedSource::Fuzzy,
        CandidateSource::HeuristicFallback => DiscoverySeedSource::HeuristicFallback,
    }
}

fn discovery_seeds(
    candidates: &[RankedDiscoveryCandidate],
    max_seeds: usize,
) -> (Vec<DiscoverySeed>, u64) {
    let mut omitted_alternatives = 0_u64;
    let seeds = candidates
        .iter()
        .take(max_seeds)
        .enumerate()
        .map(|(index, candidate)| {
            let mut alternatives = candidates
                .iter()
                .filter(|other| {
                    other.node.id != candidate.node.id
                        && other.channel_rank == candidate.channel_rank
                        && if candidate.channel_rank == 4 {
                            other.operation_root == candidate.operation_root
                                && other.relation_evidence == candidate.relation_evidence
                        } else {
                            other.score.total_cmp(&candidate.score).is_eq()
                                || source_backed_name_collision(candidate, other)
                                || calibrated_low_margin(candidate.score, other.score)
                        }
                })
                .map(|other| DiscoveryAlternative {
                    node_id: other.node.id.clone(),
                    qualified_name: other.node.qualified_name.clone(),
                    source: other.node.source.clone(),
                    score: format_discovery_score(other.score),
                })
                .collect::<Vec<_>>();
            let omitted = alternatives
                .len()
                .saturating_sub(MAX_DISCOVERY_ALTERNATIVES_PER_SEED);
            omitted_alternatives =
                omitted_alternatives.saturating_add(u64::try_from(omitted).unwrap_or(u64::MAX));
            alternatives.truncate(MAX_DISCOVERY_ALTERNATIVES_PER_SEED);
            DiscoverySeed {
                node_id: candidate.node.id.clone(),
                score: format_discovery_score(candidate.score),
                score_tier: match candidate.source {
                    DiscoverySeedSource::ExactId => DiscoveryScoreTier::ExactId,
                    DiscoverySeedSource::ExactName => DiscoveryScoreTier::ExactName,
                    DiscoverySeedSource::Alias
                    | DiscoverySeedSource::TermIndex
                    | DiscoverySeedSource::RelationSeed
                    | DiscoverySeedSource::Fuzzy
                    | DiscoverySeedSource::HeuristicFallback => DiscoveryScoreTier::Lexical,
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
        .collect();
    (seeds, omitted_alternatives)
}

fn source_backed_name_collision(
    candidate: &RankedDiscoveryCandidate,
    other: &RankedDiscoveryCandidate,
) -> bool {
    candidate.node.source.is_some()
        && other.node.source.is_some()
        && canonical_declaration_name(&candidate.node.name)
            == canonical_declaration_name(&other.node.name)
}

fn canonical_declaration_name(value: &str) -> String {
    search_tokens(value).join(" ")
}

fn calibrated_low_margin(left: f64, right: f64) -> bool {
    let scale = left.abs().max(right.abs()).max(1.0);
    (left - right).abs() <= scale * 0.01
}

fn format_discovery_score(score: f64) -> String {
    format!("{score:.6}")
}

fn edges_for_direction(
    backend: &PinnedDiscoveryBackend<'_>,
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

fn discovery_edge(edge: &EdgeRecord) -> DiscoveryEdge {
    let projected = query_edge(edge);
    DiscoveryEdge {
        id: (!projected.id.is_empty()).then_some(projected.id),
        source: projected.source,
        target: projected.target,
        kind: projected.kind,
        occurrence_rule: edge.occurrence_rule.clone(),
        relationship_site: projected.relationship_site,
        details: projected.details,
        evidence: projected.evidence,
        context: edge.context.clone(),
    }
}

fn compare_discovery_edge_occurrences(
    (left_ordinal, left): &(usize, EdgeRecord),
    (right_ordinal, right): &(usize, EdgeRecord),
) -> std::cmp::Ordering {
    match (left.id.is_empty(), right.id.is_empty()) {
        (false, false) => left
            .id
            .cmp(&right.id)
            .then_with(|| left_ordinal.cmp(right_ordinal)),
        (false, true) => std::cmp::Ordering::Less,
        (true, false) => std::cmp::Ordering::Greater,
        (true, true) => left
            .source
            .cmp(&right.source)
            .then_with(|| left.target.cmp(&right.target))
            .then_with(|| left.kind.as_str().cmp(right.kind.as_str()))
            .then_with(|| left.occurrence_rule.cmp(&right.occurrence_rule))
            .then_with(|| {
                compare_source_anchors(
                    left.relationship_site.as_ref(),
                    right.relationship_site.as_ref(),
                )
            })
            .then_with(|| left_ordinal.cmp(right_ordinal)),
    }
}

fn compare_source_anchors(
    left: Option<&compass_model::provenance::SourceAnchor>,
    right: Option<&compass_model::provenance::SourceAnchor>,
) -> std::cmp::Ordering {
    match (left, right) {
        (Some(left), Some(right)) => left
            .file
            .cmp(&right.file)
            .then_with(|| left.start_byte.cmp(&right.start_byte))
            .then_with(|| left.end_byte.cmp(&right.end_byte))
            .then_with(|| left.start_line.cmp(&right.start_line))
            .then_with(|| left.start_column.cmp(&right.start_column))
            .then_with(|| left.end_line.cmp(&right.end_line))
            .then_with(|| left.end_column.cmp(&right.end_column)),
        (Some(_), None) => std::cmp::Ordering::Less,
        (None, Some(_)) => std::cmp::Ordering::Greater,
        (None, None) => std::cmp::Ordering::Equal,
    }
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
        if let Some(seed) = response
            .seeds
            .iter_mut()
            .rev()
            .find(|seed| !seed.alternatives.is_empty())
        {
            seed.alternatives.pop();
            add_known_omission(&mut response.omissions.alternatives, 1);
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
    if let Some(existing) = slot {
        *existing = existing.saturating_add(count);
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;
    use std::path::PathBuf;
    use std::sync::atomic::AtomicBool;

    use compass_model::code_graph::{
        BuildMetadata, CommunityMetadata, EdgeKind, EdgeRecord, GraphDocument, NodeKind, NodeRecord,
    };
    use compass_model::provenance::{
        EvidenceConfidence, EvidenceOrigin, OccurrenceRule, Provenance, SourceAnchor,
    };
    use compass_model::query_contract::{
        DiscoveryDirection, DiscoveryDirectionSource, DiscoveryLimits, DiscoveryQueryRequest,
        DiscoveryScope, DiscoveryScopeKind, DiscoverySeedSource, DiscoveryTraversal,
        MAX_DISCOVERY_ALTERNATIVES_PER_SEED, MAX_DISCOVERY_CANDIDATE_NODES_READ,
        MAX_DISCOVERY_CANDIDATE_PROBES, MAX_DISCOVERY_FILTER_BYTES, MAX_DISCOVERY_FILTERS,
        MAX_DISCOVERY_QUESTION_BYTES, QueryDiagnosticCode,
    };

    use crate::code_query::{
        CodeAdjacencyIndex, CodeGraphBackend, CodeLookupIndex, FuzzyLookupCache, SearchQueryCache,
    };
    use crate::ranking::rank_search_candidates;
    use crate::recall::{CandidateSource, RecallBudget, SearchCandidatePool};
    use crate::{CodeQueryEngine, QueryEngineKind};

    use super::selective_intersection_terms;

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

    fn anchored_node(id: &str, name: &str, file: &str, line: u32) -> NodeRecord {
        let mut node = node(id, name);
        node.source = Some(SourceAnchor {
            file: file.to_owned(),
            start_byte: u64::from(line),
            end_byte: u64::from(line).saturating_add(1),
            start_line: line,
            start_column: 0,
            end_line: line,
            end_column: 1,
        });
        node
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

    fn evidenced_edge(id: &str, source: &str, target: &str, rule: &str, line: u32) -> EdgeRecord {
        let mut edge = edge(id, source, target);
        edge.occurrence_rule = OccurrenceRule::new(rule);
        edge.relationship_site = Some(SourceAnchor {
            file: "src/wiring.rs".to_owned(),
            start_byte: u64::from(line),
            end_byte: u64::from(line).saturating_add(1),
            start_line: line,
            start_column: 0,
            end_line: line,
            end_column: 1,
        });
        edge
    }

    fn heuristic_edge(id: &str, source: &str, target: &str) -> EdgeRecord {
        let mut edge = edge(id, source, target);
        edge.evidence.push(Provenance {
            origin: EvidenceOrigin::Heuristic,
            extractor: "test".to_owned(),
            confidence: EvidenceConfidence::Inferred,
            rule: None,
            anchors: Vec::new(),
            wiring_site: None,
            score: None,
            candidates: Vec::new(),
        });
        edge
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
        let connection =
            rusqlite::Connection::open_in_memory().unwrap_or_else(|_| std::process::abort());
        connection
            .execute_batch(
                r#"CREATE TABLE nodes(id TEXT PRIMARY KEY);
                   CREATE VIRTUAL TABLE node_fts USING fts5(
                     node_id UNINDEXED, name, qualified_name, aliases, kind, roles,
                     language, framework, normalized_path, source_file, community_id,
                     community_label, identifier_terms,
                     tokenize="unicode61 remove_diacritics 2 tokenchars '_'"
                   );
                   CREATE TABLE relationship_terms(
                     term TEXT NOT NULL, source_id TEXT NOT NULL,
                     PRIMARY KEY(term, source_id)
                   ) WITHOUT ROWID;
                   CREATE TABLE relationship_term_targets(
                     term TEXT NOT NULL, source_id TEXT NOT NULL, target_id TEXT NOT NULL,
                     PRIMARY KEY(source_id, term, target_id)
                   ) WITHOUT ROWID;"#,
            )
            .unwrap_or_else(|_| std::process::abort());
        for node in &graph.nodes {
            let identifier_terms = [node.name.as_str(), node.qualified_name.as_str()]
                .into_iter()
                .flat_map(compass_model::search::identifier_search_terms)
                .collect::<BTreeSet<_>>()
                .into_iter()
                .collect::<Vec<_>>()
                .join(" ");
            connection
                .execute("INSERT INTO nodes VALUES(?1)", rusqlite::params![node.id])
                .unwrap_or_else(|_| std::process::abort());
            connection
                .execute(
                    "INSERT INTO node_fts VALUES(?1,?2,?3,'',?4,'',?5,?6,'',?7,?8,?9,?10)",
                    rusqlite::params![
                        node.id,
                        node.name,
                        node.qualified_name,
                        node.kind.as_str(),
                        node.language.as_deref().unwrap_or_default(),
                        node.framework.as_deref().unwrap_or_default(),
                        node.source
                            .as_ref()
                            .map_or("", |source| source.file.as_str()),
                        node.community
                            .as_ref()
                            .map_or_else(String::new, |community| community.id.to_string()),
                        node.community
                            .as_ref()
                            .and_then(|community| community.label.as_deref())
                            .unwrap_or_default(),
                        identifier_terms,
                    ],
                )
                .unwrap_or_else(|_| std::process::abort());
        }
        for (term, source_ids) in
            compass_model::search::direct_call_source_identifier_postings(&graph)
        {
            for source_id in source_ids {
                connection
                    .execute(
                        "INSERT INTO relationship_terms VALUES(?1,?2)",
                        rusqlite::params![term, source_id],
                    )
                    .unwrap_or_else(|_| std::process::abort());
            }
        }
        for (term, source_id, target_id) in
            compass_model::search::direct_call_source_identifier_targets(&graph)
        {
            connection
                .execute(
                    "INSERT INTO relationship_term_targets VALUES(?1,?2,?3)",
                    rusqlite::params![term, source_id, target_id],
                )
                .unwrap_or_else(|_| std::process::abort());
        }
        CodeQueryEngine {
            backend: CodeGraphBackend::Materialized {
                graph: Box::new(graph),
                adjacency: Box::new(adjacency),
                lookup: Box::new(lookup),
            },
            program: None,
            connection: Some(connection),
            graph_path: PathBuf::from("graph.json"),
            index_path: PathBuf::from("index.sqlite3"),
            partial_graph_message: None,
            engine_kind: QueryEngineKind::Json,
            graph_identity: "test-graph-identity".to_owned(),
            build_generation_identity: "generation".to_owned(),
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
    fn multi_concept_discovery_rejects_isolated_generic_subword_noise()
    -> Result<(), Box<dyn std::error::Error>> {
        let engine = engine(
            vec![node("n:sync", "sync"), node("n:quantum", "quantum")],
            Vec::new(),
        );
        let mut query = request(DiscoveryDirection::Both);
        query.question = "QzxvQuantumBananaSync".to_owned();

        let response = engine.discover(query)?;

        assert!(response.seeds.is_empty());
        assert_eq!(response.stats.candidate_nodes, 0);
        assert_eq!(response.stats.candidate_probes, 2);
        assert!(
            response
                .diagnostics
                .iter()
                .any(|diagnostic| diagnostic.code == QueryDiagnosticCode::NoMatch)
        );
        Ok(())
    }

    #[test]
    fn absent_composite_identifier_is_a_proven_no_match_despite_generic_posting_noise()
    -> Result<(), Box<dyn std::error::Error>> {
        let nodes = (0..512)
            .map(|index| {
                node(
                    &format!("n:vacuum:{index:04}"),
                    &format!("VacuumWorker{index:04}"),
                )
            })
            .collect();
        let engine = engine(nodes, Vec::new());
        let mut query = request(DiscoveryDirection::Both);
        query.question = "NebulaVacuumPineappleWidget".to_owned();

        let response = engine.discover(query)?;

        assert!(response.seeds.is_empty());
        assert_eq!(response.stats.candidate_nodes, 0);
        assert_eq!(response.stats.candidate_probes, 2);
        assert!(!response.truncated);
        assert!(
            response
                .diagnostics
                .iter()
                .any(|diagnostic| diagnostic.code == QueryDiagnosticCode::NoMatch)
        );
        assert!(
            response
                .diagnostics
                .iter()
                .all(|diagnostic| diagnostic.code != QueryDiagnosticCode::BoundedTruncation)
        );
        Ok(())
    }

    #[test]
    fn multi_concept_discovery_preserves_exact_composite_identifiers()
    -> Result<(), Box<dyn std::error::Error>> {
        let engine = engine(
            vec![node("n:qzxv-quantum-banana-sync", "QzxvQuantumBananaSync")],
            Vec::new(),
        );
        let mut query = request(DiscoveryDirection::Both);
        query.question = "QzxvQuantumBananaSync".to_owned();

        let response = engine.discover(query)?;

        assert_eq!(response.seeds.len(), 1);
        assert_eq!(response.seeds[0].node_id, "n:qzxv-quantum-banana-sync");
        Ok(())
    }

    #[test]
    fn automatic_direction_uses_conservative_intent_signals()
    -> Result<(), Box<dyn std::error::Error>> {
        let engine = engine(vec![node("n:alpha", "alpha")], Vec::new());
        let cases = [
            (
                "how does alpha resolve providers",
                DiscoveryDirection::Outgoing,
                DiscoveryDirectionSource::Heuristic,
            ),
            (
                "who calls alpha",
                DiscoveryDirection::Incoming,
                DiscoveryDirectionSource::Heuristic,
            ),
            (
                "where is alpha registered by the router",
                DiscoveryDirection::Incoming,
                DiscoveryDirectionSource::Heuristic,
            ),
            (
                "where is alpha enforced at runtime",
                DiscoveryDirection::Incoming,
                DiscoveryDirectionSource::Heuristic,
            ),
            (
                "what is affected by alpha",
                DiscoveryDirection::Incoming,
                DiscoveryDirectionSource::Heuristic,
            ),
            (
                "alpha implementation flow writes to storage",
                DiscoveryDirection::Outgoing,
                DiscoveryDirectionSource::Heuristic,
            ),
            (
                "how is alpha created",
                DiscoveryDirection::Both,
                DiscoveryDirectionSource::Neutral,
            ),
            (
                "alpha architecture and coupling",
                DiscoveryDirection::Both,
                DiscoveryDirectionSource::Neutral,
            ),
            (
                "how does alpha get used by callers",
                DiscoveryDirection::Both,
                DiscoveryDirectionSource::Neutral,
            ),
        ];
        for (question, direction, source) in cases {
            let mut request = request(DiscoveryDirection::Auto);
            request.question = question.to_owned();
            let response = engine.discover(request)?;
            assert_eq!(response.selected_direction, direction, "{question}");
            assert_eq!(response.direction_source, source, "{question}");
        }
        let mut explicit = request(DiscoveryDirection::Incoming);
        explicit.question = "alpha calls beta".to_owned();
        let response = engine.discover(explicit)?;
        assert_eq!(response.selected_direction, DiscoveryDirection::Incoming);
        assert_eq!(
            response.direction_source,
            DiscoveryDirectionSource::Explicit
        );
        Ok(())
    }

    #[test]
    fn discovery_reports_the_actual_index_candidate_source()
    -> Result<(), Box<dyn std::error::Error>> {
        let cases = [
            (
                "n:alpha",
                node("n:alpha", "alpha"),
                DiscoverySeedSource::ExactId,
            ),
            (
                "alpha",
                node("n:alpha", "alpha"),
                DiscoverySeedSource::ExactName,
            ),
            (
                "alpha routing",
                node("n:alpha", "alpha"),
                DiscoverySeedSource::Alias,
            ),
            (
                "alpha",
                node("n:alpha", "alpha_handler"),
                DiscoverySeedSource::TermIndex,
            ),
            ("lits", node("n:list", "list"), DiscoverySeedSource::Fuzzy),
        ];
        for (question, candidate, expected_source) in cases {
            let engine = engine(vec![candidate], Vec::new());
            let mut query = request(DiscoveryDirection::Both);
            query.question = question.to_owned();
            let response = engine.discover(query)?;
            assert_eq!(response.seeds.len(), 1, "{question:?}");
            assert_eq!(
                response.seeds[0].candidate_source, expected_source,
                "{question:?}"
            );
        }
        Ok(())
    }

    #[test]
    fn relationship_recall_promotes_a_source_with_distinct_neighbor_terms()
    -> Result<(), Box<dyn std::error::Error>> {
        let engine = engine(
            vec![
                anchored_node("n:behavior", "CondenseSession", "src/session.rs", 20),
                anchored_node("n:checkpoint", "CheckpointID", "src/id.rs", 4),
                anchored_node(
                    "n:create",
                    "extractOrCreateSessionData",
                    "src/session.rs",
                    80,
                ),
                anchored_node("n:noise", "checkpointFixture", "tests/session.rs", 10),
            ],
            vec![
                edge("e:checkpoint", "n:behavior", "n:checkpoint"),
                edge("e:create", "n:behavior", "n:create"),
                edge("e:noise", "n:noise", "n:checkpoint"),
            ],
        );
        let mut query = request(DiscoveryDirection::Both);
        query.question = "how is a checkpoint created".to_owned();

        let response = engine.discover(query)?;

        assert_eq!(response.seeds[0].node_id, "n:behavior");
        assert_eq!(
            response.seeds[0].candidate_source,
            DiscoverySeedSource::RelationSeed
        );
        assert_eq!(response.seeds[0].matched_terms, ["checkpoint", "create"]);
        assert!(
            response.seeds[0]
                .matched_fields
                .contains(&"relationship".to_owned())
        );
        assert!(!response.seeds[0].ambiguous);
        Ok(())
    }

    #[test]
    fn relationship_recall_finds_repository_state_workflow()
    -> Result<(), Box<dyn std::error::Error>> {
        let engine = engine(
            vec![
                anchored_node("n:save", "SaveStep", "src/strategy.rs", 40),
                anchored_node("n:open", "RepositoryHandle", "src/repository.rs", 8),
                anchored_node("n:state", "StateHandle", "src/state.rs", 12),
                anchored_node("n:record", "RecordedMarker", "src/record.rs", 13),
                anchored_node("n:noise", "repositoryFixture", "tests/repository.rs", 5),
            ],
            vec![
                edge("e:open", "n:save", "n:open"),
                edge("e:state", "n:save", "n:state"),
                edge("e:record", "n:save", "n:record"),
                edge("e:noise", "n:noise", "n:open"),
            ],
        );
        let mut query = request(DiscoveryDirection::Both);
        query.question = "how is repository state recorded".to_owned();

        let response = engine.discover(query)?;

        assert_eq!(response.seeds[0].node_id, "n:save");
        assert_eq!(
            response.seeds[0].candidate_source,
            DiscoverySeedSource::RelationSeed
        );
        assert_eq!(response.seeds[0].matched_terms, ["repository", "state"]);
        assert!(!response.seeds[0].ambiguous);
        Ok(())
    }

    #[test]
    fn relationship_postings_recover_a_late_dense_source_from_an_exhaustive_sparse_driver()
    -> Result<(), Box<dyn std::error::Error>> {
        let mut nodes = vec![
            anchored_node(
                "z:behavior",
                "CondenseSession",
                "src/session/condense.rs",
                20,
            ),
            anchored_node("c:checkpoint", "CheckpointID", "src/id.rs", 4),
            anchored_node(
                "z:create",
                "extractOrCreateSessionData",
                "src/session.rs",
                80,
            ),
        ];
        let mut edges = vec![
            edge("e:checkpoint", "z:behavior", "c:checkpoint"),
            edge("e:create", "z:behavior", "z:create"),
        ];
        for index in 0..1_100 {
            let caller_id = format!("a:caller:{index:04}");
            nodes.push(anchored_node(
                &caller_id,
                &format!("fixtureCaller{index:04}"),
                if index % 2 == 0 {
                    "tests/generated/create_fixture.rs"
                } else {
                    "generated/tests/create_fixture.rs"
                },
                index + 100,
            ));
            edges.push(edge(&format!("e:dense:{index:04}"), &caller_id, "z:create"));
        }
        let ordered = engine(nodes.clone(), edges.clone());
        nodes.reverse();
        let mut shuffled_edges = edges;
        shuffled_edges.reverse();
        let shuffled = engine(nodes, shuffled_edges);
        let mut query = request(DiscoveryDirection::Both);
        query.question = "how is a checkpoint created".to_owned();

        let response = ordered.discover(query.clone())?;
        let shuffled_response = shuffled.discover(query)?;

        assert_eq!(response, shuffled_response);
        assert_eq!(response.seeds[0].node_id, "z:behavior");
        assert_eq!(response.seeds[0].matched_terms, ["checkpoint", "create"]);
        assert!(
            response.seeds[0]
                .matched_fields
                .contains(&"relationship".to_owned())
        );
        assert!(!response.seeds[0].ambiguous);
        Ok(())
    }

    #[test]
    fn observed_truncated_posting_ids_do_not_spend_membership_probes()
    -> Result<(), Box<dyn std::error::Error>> {
        let mut nodes = vec![
            anchored_node("n:checkpoint", "CheckpointID", "src/id.rs", 1),
            anchored_node("n:create", "CreateSession", "src/create.rs", 2),
        ];
        let mut edges = Vec::new();
        for index in 0..60 {
            let id = format!("a:shared:{index:04}");
            nodes.push(anchored_node(
                &id,
                "run",
                &format!("tests/shared/{index:04}.rs"),
                index + 10,
            ));
            edges.push(edge(
                &format!("e:shared:{index:04}:checkpoint"),
                &id,
                "n:checkpoint",
            ));
            edges.push(edge(
                &format!("e:shared:{index:04}:create"),
                &id,
                "n:create",
            ));
        }
        for index in 0..199 {
            let id = format!("b:create-only:{index:04}");
            nodes.push(anchored_node(
                &id,
                "run",
                &format!("tests/create-only/{index:04}.rs"),
                index + 100,
            ));
            edges.push(edge(&format!("e:create-only:{index:04}"), &id, "n:create"));
        }
        nodes.push(anchored_node(
            "zz:create-tail",
            "run",
            "tests/create-only/tail.rs",
            299,
        ));
        edges.push(edge("e:create-only:tail", "zz:create-tail", "n:create"));
        for index in 0..1_100 {
            let id = format!("m:dense:{index:04}");
            nodes.push(anchored_node(
                &id,
                "run",
                &format!("tests/dense/{index:04}.rs"),
                index + 300,
            ));
            edges.push(edge(
                &format!("e:dense:{index:04}:checkpoint"),
                &id,
                "n:checkpoint",
            ));
        }
        let engine = engine(nodes, edges);
        let mut query = request(DiscoveryDirection::Both);
        query.question = "checkpoint create".to_owned();
        query.limits.max_candidates = 256;

        let response = engine.discover(query)?;

        assert!(response.truncated);
        assert_eq!(response.stats.candidates_admitted, 188);
        assert_eq!(response.stats.candidate_probes, 130);
        Ok(())
    }

    #[test]
    fn relationship_recall_does_not_let_one_common_neighbor_beat_a_direct_symbol()
    -> Result<(), Box<dyn std::error::Error>> {
        let engine = engine(
            vec![
                anchored_node("n:direct", "checkpoint", "src/checkpoint.rs", 2),
                anchored_node("n:caller", "unrelated", "src/caller.rs", 3),
            ],
            vec![edge("e:call", "n:caller", "n:direct")],
        );
        let mut query = request(DiscoveryDirection::Both);
        query.question = "checkpoint".to_owned();

        let response = engine.discover(query)?;

        assert_eq!(response.seeds[0].node_id, "n:direct");
        assert_eq!(
            response.seeds[0].candidate_source,
            DiscoverySeedSource::ExactName
        );
        Ok(())
    }

    #[test]
    fn exact_name_beats_a_maximal_relationship_match() -> Result<(), Box<dyn std::error::Error>> {
        let engine = engine(
            vec![
                anchored_node("n:direct", "checkpoint create session", "src/direct.rs", 2),
                anchored_node("n:caller", "indirect", "src/caller.rs", 3),
                anchored_node("n:checkpoint", "CheckpointID", "src/id.rs", 4),
                anchored_node("n:create", "CreateSession", "src/create.rs", 5),
                anchored_node("n:session", "SessionData", "src/session.rs", 6),
            ],
            vec![
                edge("e:checkpoint", "n:caller", "n:checkpoint"),
                edge("e:create", "n:caller", "n:create"),
                edge("e:session", "n:caller", "n:session"),
            ],
        );
        let mut query = request(DiscoveryDirection::Both);
        query.question = "checkpoint create session".to_owned();

        let response = engine.discover(query)?;

        assert_eq!(response.seeds[0].node_id, "n:direct");
        assert_eq!(
            response.seeds[0].candidate_source,
            DiscoverySeedSource::ExactName
        );
        Ok(())
    }

    #[test]
    fn complete_exact_name_lookup_ignores_lower_channel_alias_truncation()
    -> Result<(), Box<dyn std::error::Error>> {
        let mut nodes = vec![anchored_node(
            "n:exact",
            "unique target",
            "src/target.rs",
            1,
        )];
        for index in 0..200 {
            nodes.push(anchored_node(
                &format!("n:alias:{index:03}"),
                "target",
                &format!("tests/aliases/{index:03}.rs"),
                index + 10,
            ));
        }
        let engine = engine(nodes, Vec::new());
        let mut query = request(DiscoveryDirection::Both);
        query.question = "unique target".to_owned();

        let response = engine.discover(query)?;

        assert!(response.truncated);
        assert_eq!(response.seeds[0].node_id, "n:exact");
        assert_eq!(
            response.seeds[0].candidate_source,
            DiscoverySeedSource::ExactName
        );
        assert!(!response.seeds[0].ambiguous);
        Ok(())
    }

    #[test]
    fn selective_intersections_are_deterministic_and_bounded() {
        let concepts = (0..12)
            .map(|index| format!("term{index:02}"))
            .collect::<Vec<_>>();

        let first = selective_intersection_terms(&concepts);
        let second = selective_intersection_terms(&concepts);

        assert_eq!(first, second);
        assert_eq!(first.len(), super::MAX_DISCOVERY_INTERSECTION_PROBES);
        assert!(first.iter().all(|intersection| intersection.len() == 2));
    }

    #[test]
    fn selective_intersections_prioritize_specific_behavior_pairs() {
        let concepts = vec![
            "http".to_owned(),
            "process".to_owned(),
            "request".to_owned(),
        ];

        let intersections = selective_intersection_terms(&concepts);

        assert_eq!(
            intersections,
            vec![
                vec!["process".to_owned(), "request".to_owned()],
                vec!["http".to_owned(), "process".to_owned()],
                vec!["http".to_owned(), "request".to_owned()],
                concepts,
            ]
        );
    }

    #[test]
    fn equal_relationship_callers_are_reported_as_ambiguous()
    -> Result<(), Box<dyn std::error::Error>> {
        let mut nodes = vec![
            anchored_node("n:caller:a", "run", "src/a.rs", 2),
            anchored_node("n:caller:b", "run", "src/b.rs", 3),
            anchored_node("n:checkpoint", "CheckpointID", "src/id.rs", 4),
            anchored_node("n:create", "CreateSession", "src/create.rs", 5),
        ];
        let mut edges = vec![
            edge("e:a:checkpoint", "n:caller:a", "n:checkpoint"),
            edge("e:a:create", "n:caller:a", "n:create"),
            edge("e:b:checkpoint", "n:caller:b", "n:checkpoint"),
            edge("e:b:create", "n:caller:b", "n:create"),
        ];
        let ordered = engine(nodes.clone(), edges.clone());
        let mut query = request(DiscoveryDirection::Both);
        query.question = "checkpoint create".to_owned();

        let response = ordered.discover(query.clone())?;
        nodes.reverse();
        edges.reverse();
        let reversed = engine(nodes, edges).discover(query)?;

        assert_eq!(response, reversed);
        assert!(response.seeds[0].ambiguous);
        assert_eq!(response.seeds[0].alternatives.len(), 1);
        assert_eq!(response.seeds[0].alternatives[0].node_id, "n:caller:b");
        Ok(())
    }

    #[test]
    fn direct_concept_intersection_precedes_ranked_relationship_evidence()
    -> Result<(), Box<dyn std::error::Error>> {
        let mut nodes = vec![
            anchored_node(
                "z:production-four",
                "createWorkflow",
                "src/workflow/four.rs",
                1,
            ),
            anchored_node(
                "a:production-three",
                "createWorkflow",
                "src/workflow/three.rs",
                2,
            ),
            anchored_node("0:test-five", "createWorkflow", "tests/workflow_test.rs", 3),
            anchored_node("t:both", "CheckpointCreate", "src/targets.rs", 10),
            anchored_node("t:checkpoint:1", "CheckpointID", "src/targets.rs", 11),
            anchored_node("t:checkpoint:2", "CheckpointStore", "src/targets.rs", 12),
            anchored_node("t:create:1", "CreateSession", "src/targets.rs", 13),
            anchored_node("t:create:2", "CreateData", "src/targets.rs", 14),
            anchored_node("t:create:3", "CreateCommit", "src/targets.rs", 15),
        ];
        let mut edges = vec![
            edge("e:p4:cp1", "z:production-four", "t:checkpoint:1"),
            edge("e:p4:cp2", "z:production-four", "t:checkpoint:2"),
            edge("e:p4:c1", "z:production-four", "t:create:1"),
            edge("e:p4:c2", "z:production-four", "t:create:2"),
            edge("e:p3:both", "a:production-three", "t:both"),
            edge("e:p3:cp", "a:production-three", "t:checkpoint:1"),
            edge("e:p3:create", "a:production-three", "t:create:1"),
            edge("e:t5:both", "0:test-five", "t:both"),
            edge("e:t5:cp1", "0:test-five", "t:checkpoint:1"),
            edge("e:t5:create1", "0:test-five", "t:create:1"),
            edge("e:t5:create2", "0:test-five", "t:create:2"),
            edge("e:t5:create3", "0:test-five", "t:create:3"),
        ];
        let ordered = engine(nodes.clone(), edges.clone());
        let mut query = request(DiscoveryDirection::Both);
        query.question = "checkpoint create".to_owned();
        let response = ordered.discover(query.clone())?;
        nodes.reverse();
        edges.reverse();
        let reversed = engine(nodes, edges).discover(query)?;

        assert_eq!(response, reversed);
        assert_eq!(response.seeds[0].node_id, "t:both");
        assert!(!response.seeds[0].ambiguous);
        assert_eq!(response.seeds[1].node_id, "z:production-four");
        assert_eq!(response.seeds[2].node_id, "a:production-three");
        Ok(())
    }

    #[test]
    fn scope_filtering_precedes_relationship_promotion_cap()
    -> Result<(), Box<dyn std::error::Error>> {
        let mut nodes = vec![
            anchored_node("n:checkpoint", "CheckpointID", "src/shared/id.rs", 4),
            anchored_node("n:create", "CreateSession", "src/shared/create.rs", 5),
            anchored_node("z:in-scope", "run", "src/workflow/run.rs", 8),
        ];
        let mut edges = vec![
            edge("e:z:checkpoint", "z:in-scope", "n:checkpoint"),
            edge("e:z:create", "z:in-scope", "n:create"),
        ];
        for index in 0..128 {
            let id = format!("a:out-of-scope:{index:03}");
            nodes.push(anchored_node(
                &id,
                "run",
                &format!("tests/generated/{index:03}.rs"),
                index + 10,
            ));
            edges.push(edge(
                &format!("e:a:{index:03}:checkpoint"),
                &id,
                "n:checkpoint",
            ));
            edges.push(edge(&format!("e:a:{index:03}:create"), &id, "n:create"));
        }
        let engine = engine(nodes, edges);
        let mut query = request(DiscoveryDirection::Both);
        query.question = "checkpoint create".to_owned();
        query.limits.max_candidates = 256;
        query.scope = vec![DiscoveryScope {
            kind: DiscoveryScopeKind::Source,
            value: "src/workflow".to_owned(),
        }];

        let response = engine.discover(query)?;

        assert_eq!(response.seeds[0].node_id, "z:in-scope");
        assert!(!response.seeds[0].ambiguous);
        Ok(())
    }

    #[test]
    fn relationship_promotion_cap_uses_canonical_ranking_before_admission()
    -> Result<(), Box<dyn std::error::Error>> {
        let mut nodes = vec![
            anchored_node("n:checkpoint", "CheckpointID", "src/shared/id.rs", 4),
            anchored_node("n:create", "CreateSession", "src/shared/create.rs", 5),
            anchored_node("z:production", "run", "src/workflow/run.rs", 8),
        ];
        let mut edges = vec![
            edge("e:z:checkpoint", "z:production", "n:checkpoint"),
            edge("e:z:create", "z:production", "n:create"),
        ];
        for index in 0..128 {
            let id = format!("a:test-helper:{index:03}");
            nodes.push(anchored_node(
                &id,
                "run",
                &format!("tests/generated/{index:03}.rs"),
                index + 10,
            ));
            edges.push(edge(
                &format!("e:a:{index:03}:checkpoint"),
                &id,
                "n:checkpoint",
            ));
            edges.push(edge(&format!("e:a:{index:03}:create"), &id, "n:create"));
        }
        let engine = engine(nodes, edges);
        let mut query = request(DiscoveryDirection::Both);
        query.question = "checkpoint create".to_owned();
        query.limits.max_candidates = 256;

        let response = engine.discover(query)?;

        assert_eq!(response.seeds[0].node_id, "z:production");
        assert_eq!(
            response.seeds[0].candidate_source,
            DiscoverySeedSource::RelationSeed
        );
        Ok(())
    }

    #[test]
    fn lower_channel_alias_truncation_does_not_make_a_proven_relationship_seed_ambiguous()
    -> Result<(), Box<dyn std::error::Error>> {
        let mut nodes = vec![
            anchored_node("n:behavior", "run", "src/workflow/run.rs", 1),
            anchored_node("n:checkpoint", "CheckpointID", "src/id.rs", 2),
            anchored_node("n:create", "CreateSession", "src/create.rs", 3),
        ];
        for index in 0..200 {
            nodes.push(anchored_node(
                &format!("n:alias:{index:03}"),
                "checkpoint",
                &format!("tests/aliases/{index:03}.rs"),
                index + 10,
            ));
        }
        let engine = engine(
            nodes,
            vec![
                edge("e:checkpoint", "n:behavior", "n:checkpoint"),
                edge("e:create", "n:behavior", "n:create"),
            ],
        );
        let mut query = request(DiscoveryDirection::Both);
        query.question = "checkpoint create".to_owned();

        let response = engine.discover(query)?;

        assert!(response.truncated);
        assert_eq!(response.seeds[0].node_id, "n:behavior");
        assert!(!response.seeds[0].ambiguous);
        Ok(())
    }

    #[test]
    fn relationship_recall_is_calls_only_nonrecursive_and_direction_gated()
    -> Result<(), Box<dyn std::error::Error>> {
        let nodes = vec![
            anchored_node("n:behavior", "CondenseSession", "src/session.rs", 20),
            anchored_node("n:outer", "OuterWorkflow", "src/workflow.rs", 2),
            anchored_node("n:checkpoint", "CheckpointID", "src/id.rs", 4),
            anchored_node(
                "n:create",
                "extractOrCreateSessionData",
                "src/session.rs",
                80,
            ),
        ];
        let edges = vec![
            edge("e:checkpoint", "n:behavior", "n:checkpoint"),
            edge("e:create", "n:behavior", "n:create"),
            edge("e:outer", "n:outer", "n:behavior"),
        ];
        let engine = engine(nodes, edges);
        let mut query = request(DiscoveryDirection::Both);
        query.question = "checkpoint create".to_owned();
        query.limits.max_candidates = 2;
        let response = engine.discover(query)?;
        assert_eq!(response.seeds[0].node_id, "n:behavior");
        assert!(response.seeds.iter().all(|seed| seed.node_id != "n:outer"));

        let mut incoming = request(DiscoveryDirection::Incoming);
        incoming.question = "checkpoint create".to_owned();
        assert!(
            engine
                .discover(incoming)?
                .seeds
                .iter()
                .all(|seed| seed.candidate_source != DiscoverySeedSource::RelationSeed)
        );
        Ok(())
    }

    #[test]
    fn relationship_recall_excludes_heuristic_calls_and_respects_edge_budget()
    -> Result<(), Box<dyn std::error::Error>> {
        let nodes = vec![
            anchored_node("n:behavior", "CondenseSession", "src/session.rs", 20),
            anchored_node("n:checkpoint", "CheckpointID", "src/id.rs", 4),
            anchored_node(
                "n:create",
                "extractOrCreateSessionData",
                "src/session.rs",
                80,
            ),
        ];
        let heuristic = engine(
            nodes.clone(),
            vec![
                heuristic_edge("e:checkpoint", "n:behavior", "n:checkpoint"),
                heuristic_edge("e:create", "n:behavior", "n:create"),
            ],
        );
        let mut query = request(DiscoveryDirection::Both);
        query.question = "checkpoint create".to_owned();
        assert!(
            heuristic
                .discover(query.clone())?
                .seeds
                .iter()
                .all(|seed| seed.candidate_source != DiscoverySeedSource::RelationSeed)
        );

        let bounded = engine(
            nodes,
            vec![
                edge("e:checkpoint", "n:behavior", "n:checkpoint"),
                edge("e:create", "n:behavior", "n:create"),
            ],
        );
        query.limits.max_expanded_relationships = 1;
        let response = bounded.discover(query)?;
        assert!(response.truncated);
        assert!(response.stats.expanded_relationships <= 1);
        assert!(
            response
                .seeds
                .iter()
                .all(|seed| seed.candidate_source != DiscoverySeedSource::RelationSeed)
        );
        Ok(())
    }

    #[test]
    fn relationship_recall_never_claims_completeness_before_all_concepts_are_examined()
    -> Result<(), Box<dyn std::error::Error>> {
        let engine = engine(
            vec![
                anchored_node("n:behavior", "run", "src/run.rs", 1),
                anchored_node("n:checkpoint", "CheckpointID", "src/id.rs", 2),
                anchored_node("n:create", "CreateSession", "src/create.rs", 3),
                anchored_node("n:state", "SessionState", "src/state.rs", 4),
            ],
            vec![
                edge("e:checkpoint", "n:behavior", "n:checkpoint"),
                edge("e:create", "n:behavior", "n:create"),
                edge("e:state", "n:behavior", "n:state"),
            ],
        );
        let mut query = request(DiscoveryDirection::Both);
        query.question = "checkpoint create state".to_owned();
        query.limits.max_expanded_relationships = 255;

        let response = engine.discover(query)?;

        assert!(response.truncated);
        assert!(response.seeds.iter().all(|seed| seed.ambiguous));
        assert!(
            response
                .seeds
                .iter()
                .all(|seed| seed.candidate_source != DiscoverySeedSource::RelationSeed)
        );
        assert!(response.stats.expanded_relationships <= 255);
        Ok(())
    }

    #[test]
    fn relationship_recall_uses_distinct_terms_to_break_same_name_noise()
    -> Result<(), Box<dyn std::error::Error>> {
        let engine = engine(
            vec![
                anchored_node("n:expected", "run", "src/expected.rs", 10),
                anchored_node("n:noise", "run", "src/noise.rs", 10),
                anchored_node("n:checkpoint:a", "CheckpointID", "src/a.rs", 1),
                anchored_node("n:checkpoint:b", "CheckpointID", "src/b.rs", 1),
                anchored_node("n:create", "createSession", "src/create.rs", 1),
            ],
            vec![
                edge("e:expected-checkpoint", "n:expected", "n:checkpoint:a"),
                edge("e:expected-create", "n:expected", "n:create"),
                edge("e:noise-checkpoint", "n:noise", "n:checkpoint:b"),
            ],
        );
        let mut query = request(DiscoveryDirection::Both);
        query.question = "checkpoint created".to_owned();

        let response = engine.discover(query)?;

        assert_eq!(response.seeds[0].node_id, "n:expected");
        assert!(!response.seeds[0].ambiguous);
        Ok(())
    }

    #[test]
    fn indexed_seeds_match_an_exhaustive_small_graph_oracle()
    -> Result<(), Box<dyn std::error::Error>> {
        let nodes = vec![
            node("n:alpha:a", "alpha"),
            node("n:alpha:b", "alpha"),
            node("n:beta", "beta"),
        ];
        let engine = engine(nodes.clone(), Vec::new());
        let response = engine.discover(request(DiscoveryDirection::Both))?;

        let mut exhaustive = SearchCandidatePool::new(RecallBudget {
            max_total_candidates: 256,
            max_per_source: 256,
            max_fuzzy_candidates: 16,
        });
        for candidate in nodes {
            if candidate.name == "alpha" || candidate.qualified_name == "alpha" {
                let _ = exhaustive.add(CandidateSource::ExactName, candidate.clone());
                let _ = exhaustive.add(CandidateSource::Alias, candidate.clone());
                let _ = exhaustive.add(CandidateSource::TermIndex, candidate);
            }
        }
        let expected_ranked =
            rank_search_candidates("alpha", &["alpha".to_owned()], exhaustive.into_vec(), 256);
        let expected = expected_ranked
            .iter()
            .map(|candidate| {
                (
                    candidate.node_id.clone(),
                    super::format_discovery_score(candidate.score),
                    expected_ranked.iter().any(|other| {
                        other.node_id != candidate.node_id
                            && other.score.total_cmp(&candidate.score).is_eq()
                    }),
                )
            })
            .collect::<Vec<_>>();
        let actual = response
            .seeds
            .iter()
            .map(|seed| (seed.node_id.clone(), seed.score.clone(), seed.ambiguous))
            .collect::<Vec<_>>();
        assert_eq!(actual, expected);
        assert!(
            response
                .seeds
                .iter()
                .all(|seed| seed.candidate_source == DiscoverySeedSource::ExactName)
        );
        Ok(())
    }

    #[test]
    fn indexed_candidate_work_is_bounded_independent_of_graph_size()
    -> Result<(), Box<dyn std::error::Error>> {
        let mut observed = Vec::new();
        for noise_count in [8_usize, 4_096] {
            let mut nodes = vec![node("n:alpha", "alpha")];
            for index in 0..noise_count {
                nodes.push(node(
                    &format!("n:noise:{index:05}"),
                    &format!("unrelated_{index:05}"),
                ));
            }
            let engine = engine(nodes, Vec::new());
            let response = engine.discover(request(DiscoveryDirection::Both))?;
            assert!(response.stats.candidate_nodes <= MAX_DISCOVERY_CANDIDATE_NODES_READ);
            assert!(response.stats.candidate_probes <= MAX_DISCOVERY_CANDIDATE_PROBES);
            assert!(
                response.stats.candidates_admitted <= u64::from(response.limits.max_candidates)
            );
            observed.push((
                response.stats.candidate_nodes,
                response.stats.candidates_admitted,
            ));
        }
        assert_eq!(observed[0], observed[1]);
        Ok(())
    }

    #[test]
    fn scoped_recall_uses_the_shared_read_ceiling_before_admission()
    -> Result<(), Box<dyn std::error::Error>> {
        let nodes = (0..400)
            .map(|index| node(&format!("n:{index:02}"), "alpha"))
            .collect::<Vec<_>>();
        let engine = engine(nodes, Vec::new());
        let mut query = request(DiscoveryDirection::Both);
        query.limits.max_candidates = 1;
        query.scope = vec![DiscoveryScope {
            kind: DiscoveryScopeKind::Node,
            value: "n:300".to_owned(),
        }];
        let response = engine.discover(query)?;
        assert_eq!(response.seeds.len(), 1);
        assert_eq!(response.seeds[0].node_id, "n:300");
        assert_eq!(response.stats.candidates_admitted, 1);
        assert!(response.stats.candidate_nodes > response.stats.candidates_admitted);
        Ok(())
    }

    #[test]
    fn repeated_scopes_are_canonical_deduplicated_and_or_combined()
    -> Result<(), Box<dyn std::error::Error>> {
        let engine = engine(
            vec![
                anchored_node("n:prod", "alpha", "src/認証/mod.rs", 1),
                anchored_node("n:test", "alpha", "tests/auth.rs", 2),
                anchored_node("n:other", "alpha", "vendor/auth.rs", 3),
            ],
            Vec::new(),
        );
        let mut query = request(DiscoveryDirection::Both);
        query.scope = vec![
            DiscoveryScope {
                kind: DiscoveryScopeKind::Source,
                value: "tests/".to_owned(),
            },
            DiscoveryScope {
                kind: DiscoveryScopeKind::Source,
                value: "src\\認証".to_owned(),
            },
            DiscoveryScope {
                kind: DiscoveryScopeKind::Source,
                value: "tests".to_owned(),
            },
        ];
        let response = engine.discover(query)?;
        assert_eq!(
            response.scope,
            [
                DiscoveryScope {
                    kind: DiscoveryScopeKind::Source,
                    value: "src/認証".to_owned(),
                },
                DiscoveryScope {
                    kind: DiscoveryScopeKind::Source,
                    value: "tests".to_owned(),
                },
            ]
        );
        assert_eq!(
            response
                .seeds
                .iter()
                .map(|seed| seed.node_id.as_str())
                .collect::<BTreeSet<_>>(),
            BTreeSet::from(["n:prod", "n:test"])
        );
        Ok(())
    }

    #[test]
    fn unknown_and_ambiguous_scopes_are_typed_rejections() -> Result<(), Box<dyn std::error::Error>>
    {
        let mut first = anchored_node("n:a", "run", "src/a.rs", 1);
        first.qualified_name = "pkg::run".to_owned();
        let mut second = anchored_node("n:b", "run", "src/b.rs", 2);
        second.qualified_name = "pkg::run".to_owned();
        let engine = engine(vec![first, second], Vec::new());

        let mut unknown = request(DiscoveryDirection::Both);
        unknown.scope = vec![DiscoveryScope {
            kind: DiscoveryScopeKind::Source,
            value: "missing".to_owned(),
        }];
        let error = engine
            .discover(unknown)
            .err()
            .unwrap_or_else(|| std::process::abort());
        assert_eq!(error.code(), "unknown_discovery_scope");
        assert!(
            error
                .message()
                .contains("existing normalized source path or path prefix")
        );

        let mut ambiguous = request(DiscoveryDirection::Both);
        ambiguous.scope = vec![DiscoveryScope {
            kind: DiscoveryScopeKind::Node,
            value: "pkg::run".to_owned(),
        }];
        let error = engine
            .discover(ambiguous)
            .err()
            .unwrap_or_else(|| std::process::abort());
        assert_eq!(error.code(), "ambiguous_discovery_scope");
        assert!(error.message().contains("n:a"));
        assert!(error.message().contains("n:b"));
        assert!(error.message().contains("candidate IDs"));
        assert!(error.message().contains("use an exact node ID"));
        Ok(())
    }

    #[test]
    fn community_label_ambiguity_and_package_prefixes_are_resolved_explicitly()
    -> Result<(), Box<dyn std::error::Error>> {
        let mut first = anchored_node("n:a", "login", "src/auth/login.rs", 1);
        first.qualified_name = "crate::auth::login".to_owned();
        first.community = Some(CommunityMetadata {
            id: 1,
            label: Some("core".to_owned()),
            score: None,
            color: None,
        });
        let mut second = anchored_node("n:b", "logout", "src/auth/logout.rs", 2);
        second.qualified_name = "crate::auth::logout".to_owned();
        second.community = Some(CommunityMetadata {
            id: 2,
            label: Some("core".to_owned()),
            score: None,
            color: None,
        });
        let engine = engine(vec![first, second], Vec::new());

        let mut ambiguous = request(DiscoveryDirection::Both);
        ambiguous.scope = vec![DiscoveryScope {
            kind: DiscoveryScopeKind::Community,
            value: "core".to_owned(),
        }];
        let error = engine
            .discover(ambiguous)
            .err()
            .unwrap_or_else(|| std::process::abort());
        assert_eq!(error.code(), "ambiguous_discovery_scope");
        assert!(error.message().contains('1'));
        assert!(error.message().contains('2'));

        let mut package = request(DiscoveryDirection::Both);
        package.question = "login".to_owned();
        package.scope = vec![DiscoveryScope {
            kind: DiscoveryScopeKind::Package,
            value: "::crate::auth::".to_owned(),
        }];
        let response = engine.discover(package)?;
        assert_eq!(response.scope[0].value, "crate::auth");
        assert_eq!(response.seeds[0].node_id, "n:a");
        Ok(())
    }

    #[test]
    fn context_aliases_are_validated_before_graph_filtering()
    -> Result<(), Box<dyn std::error::Error>> {
        let engine = engine(
            vec![node("n:alpha", "alpha"), node("n:callee", "callee")],
            vec![edge("e:call", "n:alpha", "n:callee")],
        );
        let mut valid_empty = request(DiscoveryDirection::Outgoing);
        valid_empty.relation_contexts = vec!["imports".to_owned()];
        let response = engine.discover(valid_empty)?;
        assert_eq!(response.relation_contexts, ["import"]);
        assert!(response.edges.is_empty());

        for invalid in ["cal", "   "] {
            let mut query = request(DiscoveryDirection::Both);
            query.relation_contexts = vec![invalid.to_owned()];
            let error = engine
                .discover(query)
                .err()
                .unwrap_or_else(|| std::process::abort());
            assert_eq!(error.kind(), crate::QueryErrorKind::InvalidParameter);
        }
        Ok(())
    }

    #[test]
    fn ambiguity_uses_full_ranking_and_bounds_alternatives()
    -> Result<(), Box<dyn std::error::Error>> {
        let nodes = (0..12)
            .map(|index| {
                anchored_node(
                    &format!("n:{index:02}"),
                    "alpha",
                    if index == 0 {
                        "src/alpha.rs"
                    } else {
                        "tests/alpha.rs"
                    },
                    index + 1,
                )
            })
            .collect::<Vec<_>>();
        let engine = engine(nodes, Vec::new());
        let mut query = request(DiscoveryDirection::Both);
        query.limits.max_seeds = 1;
        let first = engine.discover(query.clone())?;
        let second = engine.discover(query)?;
        assert_eq!(first.seeds.len(), 1);
        assert!(first.seeds[0].ambiguous);
        assert_eq!(
            first.seeds[0].alternatives.len(),
            MAX_DISCOVERY_ALTERNATIVES_PER_SEED
        );
        assert_eq!(first.omissions.alternatives, Some(3));
        assert!(first.truncated);
        assert_eq!(serde_json::to_vec(&first)?, serde_json::to_vec(&second)?);
        Ok(())
    }

    #[test]
    fn broad_scope_does_not_hide_a_late_unique_lexical_match()
    -> Result<(), Box<dyn std::error::Error>> {
        let mut nodes = (0..12_900)
            .map(|index| {
                anchored_node(
                    &format!("n:{index:05}"),
                    &format!("unrelated_{index:05}"),
                    &format!("src/module_{index:05}.rs"),
                    u32::try_from(index).unwrap_or(u32::MAX).saturating_add(1),
                )
            })
            .collect::<Vec<_>>();
        nodes.push(anchored_node(
            "z:target",
            "unique_needle",
            "src/最後.rs",
            13_001,
        ));
        let engine = engine(nodes, Vec::new());
        let mut query = request(DiscoveryDirection::Both);
        query.question = "unique_needle".to_owned();
        query.scope = vec![DiscoveryScope {
            kind: DiscoveryScopeKind::Source,
            value: "src".to_owned(),
        }];
        let response = engine.discover(query)?;
        assert_eq!(response.seeds[0].node_id, "z:target");
        assert!(response.stats.candidate_nodes <= MAX_DISCOVERY_CANDIDATE_NODES_READ);
        assert!(response.stats.candidate_probes <= MAX_DISCOVERY_CANDIDATE_PROBES);
        Ok(())
    }

    #[test]
    fn maximum_term_question_stays_inside_read_and_probe_budgets()
    -> Result<(), Box<dyn std::error::Error>> {
        let engine = engine(vec![node("n:alpha", "alpha")], Vec::new());
        let mut query = request(DiscoveryDirection::Both);
        query.question = (0..32)
            .map(|index| format!("missingterm{index:02}"))
            .collect::<Vec<_>>()
            .join(" ");
        let response = engine.discover(query)?;
        assert!(response.stats.candidate_nodes <= MAX_DISCOVERY_CANDIDATE_NODES_READ);
        assert!(response.stats.candidate_probes <= MAX_DISCOVERY_CANDIDATE_PROBES);
        assert!(response.stats.candidate_probes >= 34);
        assert!(response.stats.candidate_probes <= 114);
        assert_eq!(response.stats.candidates_admitted, 0);
        Ok(())
    }

    #[test]
    fn expired_guard_stops_indexed_recall_before_a_probe() {
        let engine = engine(vec![node("n:alpha", "alpha")], Vec::new());
        let guard = super::DiscoveryGuard {
            deadline: std::time::Instant::now()
                .checked_sub(std::time::Duration::from_millis(1))
                .unwrap_or_else(std::time::Instant::now),
            cancelled: None,
        };
        let backend = engine
            .backend
            .pin_discovery()
            .unwrap_or_else(|_| std::process::abort());
        let error = engine
            .indexed_candidates(
                &backend,
                "alpha",
                &[],
                DiscoveryDirection::Both,
                &DiscoveryLimits::default(),
                &guard,
            )
            .err()
            .unwrap_or_else(|| std::process::abort());
        assert_eq!(error.kind(), crate::QueryErrorKind::Timeout);
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
    fn node_cap_makes_omission_counts_unknown() -> Result<(), Box<dyn std::error::Error>> {
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
        assert_eq!(response.omissions.nodes, None);
        assert_eq!(response.omissions.expanded_relationships, None);
        assert!(response.truncated);
        assert!(
            response
                .diagnostics
                .iter()
                .any(|diagnostic| diagnostic.code == QueryDiagnosticCode::BoundedTruncation)
        );
        Ok(())
    }

    #[test]
    fn node_cap_does_not_claim_exact_omissions_beyond_an_unvisited_node()
    -> Result<(), Box<dyn std::error::Error>> {
        let engine = engine(
            vec![
                node("n:alpha", "alpha"),
                node("n:middle", "middle"),
                node("n:leaf", "leaf"),
            ],
            vec![
                edge("e:first", "n:alpha", "n:middle"),
                edge("e:second", "n:middle", "n:leaf"),
            ],
        );
        let mut bounded_request = request(DiscoveryDirection::Outgoing);
        bounded_request.limits.max_nodes = 1;
        bounded_request.limits.max_depth = 2;
        let response = engine.discover(bounded_request)?;
        assert_eq!(
            response
                .nodes
                .iter()
                .map(|node| node.id.as_str())
                .collect::<Vec<_>>(),
            vec!["n:alpha"]
        );
        assert_eq!(response.omissions.nodes, None);
        assert_eq!(response.omissions.expanded_relationships, None);
        assert!(response.truncated);
        assert!(
            response
                .diagnostics
                .iter()
                .any(|diagnostic| diagnostic.code == QueryDiagnosticCode::BoundedTruncation)
        );
        Ok(())
    }

    #[test]
    fn node_cap_stops_endpoint_expansion_but_preserves_selected_subgraph_edges()
    -> Result<(), Box<dyn std::error::Error>> {
        let mut nodes = vec![node("n:alpha", "alpha")];
        let mut edges = Vec::new();
        for index in 0..1_024 {
            let id = format!("n:leaf:{index:04}");
            nodes.push(node(&id, &format!("leaf_{index:04}")));
            edges.push(edge(&format!("e:{index:04}"), "n:alpha", &id));
        }
        let engine = engine(nodes, edges);
        let mut bounded_request = request(DiscoveryDirection::Both);
        bounded_request.limits.max_nodes = 2;
        bounded_request.limits.max_edges = 1_000;
        bounded_request.limits.max_expanded_relationships = 4_096;

        let response = engine.discover(bounded_request)?;

        assert_eq!(response.nodes.len(), 2);
        assert_eq!(response.stats.visited_nodes, 2);
        assert!(response.stats.expanded_relationships <= 1_027);
        assert_eq!(response.edges.len(), 1);
        assert_eq!(response.omissions.edges, Some(0));
        assert_eq!(response.omissions.expanded_relationships, None);
        assert!(response.truncated);
        Ok(())
    }

    #[test]
    fn response_byte_trimming_preserves_unknown_node_and_expansion_omissions()
    -> Result<(), Box<dyn std::error::Error>> {
        let engine = engine(
            vec![
                node("n:alpha", "alpha"),
                node("n:first", "first"),
                node("n:second", "second"),
                node("n:hidden", "hidden"),
            ],
            vec![
                edge("e:first", "n:alpha", "n:first"),
                edge("e:second", "n:alpha", "n:second"),
                edge("e:hidden", "n:second", "n:hidden"),
            ],
        );
        let mut node_bounded = request(DiscoveryDirection::Outgoing);
        node_bounded.limits.max_nodes = 2;
        node_bounded.limits.max_depth = 2;
        let before_byte_trim = engine.discover(node_bounded.clone())?;
        assert_eq!(before_byte_trim.omissions.nodes, None);
        assert_eq!(before_byte_trim.omissions.expanded_relationships, None);
        assert!(!before_byte_trim.edges.is_empty());

        node_bounded.limits.max_response_bytes = u64::try_from(
            serde_json::to_vec(&before_byte_trim)?
                .len()
                .saturating_sub(1),
        )
        .unwrap_or(u64::MAX);
        let response = engine.discover(node_bounded)?;
        assert!(response.truncated);
        assert_eq!(response.omissions.nodes, None);
        assert_eq!(response.omissions.expanded_relationships, None);
        assert!(serde_json::to_vec(&response)?.len() as u64 <= response.limits.max_response_bytes);
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
        assert_eq!(first.omissions.candidates, Some(0));
        assert_eq!(first.omissions.nodes, Some(0));
        assert_eq!(first.omissions.edges, Some(0));
        assert_eq!(first.omissions.expanded_relationships, Some(0));
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
    fn discovery_enforces_shared_query_boundaries_before_recall()
    -> Result<(), Box<dyn std::error::Error>> {
        let engine = engine(vec![node("n:alpha", "alpha")], Vec::new());
        let mut at_limit = request(DiscoveryDirection::Both);
        at_limit.question = "x".repeat(MAX_DISCOVERY_QUESTION_BYTES);
        let response = engine.discover(at_limit)?;
        assert!(response.seeds.is_empty());

        let mut over_bytes = request(DiscoveryDirection::Both);
        over_bytes.question = "x".repeat(MAX_DISCOVERY_QUESTION_BYTES + 1);
        let error = engine
            .discover(over_bytes)
            .err()
            .unwrap_or_else(|| std::process::abort());
        assert_eq!(error.code(), "invalid_discovery_question");

        let mut over_terms = request(DiscoveryDirection::Both);
        over_terms.question = (0..33)
            .map(|index| format!("term{index:02}"))
            .collect::<Vec<_>>()
            .join(" ");
        let error = engine
            .discover(over_terms)
            .err()
            .unwrap_or_else(|| std::process::abort());
        assert_eq!(error.code(), "too_many_search_terms");
        Ok(())
    }

    #[test]
    fn boolean_only_question_is_a_deterministic_no_match() -> Result<(), Box<dyn std::error::Error>>
    {
        let engine = engine(vec![node("n:alpha", "alpha")], Vec::new());
        let mut query = request(DiscoveryDirection::Both);
        query.question = "AND OR NOT NEAR".to_owned();
        let first = engine.discover(query.clone())?;
        let second = engine.discover(query)?;
        assert_eq!(serde_json::to_vec(&first)?, serde_json::to_vec(&second)?);
        assert!(first.seeds.is_empty());
        assert_eq!(first.stats.candidate_probes, 0);
        assert_eq!(first.stats.candidate_nodes, 0);
        assert_eq!(first.stats.candidates_admitted, 0);
        assert!(
            first
                .diagnostics
                .iter()
                .any(|diagnostic| diagnostic.code == QueryDiagnosticCode::NoMatch)
        );
        Ok(())
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

    #[test]
    fn direction_and_parallel_edge_evidence_are_preserved() -> Result<(), Box<dyn std::error::Error>>
    {
        let engine = engine(
            vec![
                node("n:alpha", "alpha"),
                node("n:caller", "caller"),
                node("n:callee", "callee"),
            ],
            vec![
                edge("e:incoming:first", "n:caller", "n:alpha"),
                edge("e:incoming:second", "n:caller", "n:alpha"),
                edge("e:outgoing", "n:alpha", "n:callee"),
            ],
        );
        let incoming = engine.discover(request(DiscoveryDirection::Incoming))?;
        assert_eq!(incoming.edges.len(), 2);
        assert!(
            incoming
                .edges
                .iter()
                .all(|edge| edge.source == "n:caller" && edge.target == "n:alpha")
        );

        let outgoing = engine.discover(request(DiscoveryDirection::Outgoing))?;
        assert_eq!(outgoing.edges.len(), 1);
        assert_eq!(outgoing.edges[0].id.as_deref(), Some("e:outgoing"));

        let both = engine.discover(request(DiscoveryDirection::Both))?;
        assert_eq!(both.edges.len(), 3);
        assert_eq!(
            both.edges
                .iter()
                .map(|edge| edge.id.as_deref())
                .collect::<Vec<_>>(),
            [
                Some("e:incoming:first"),
                Some("e:incoming:second"),
                Some("e:outgoing")
            ]
        );
        Ok(())
    }

    #[test]
    fn empty_id_parallel_edges_remain_distinct_without_synthesized_ids()
    -> Result<(), Box<dyn std::error::Error>> {
        let engine = engine(
            vec![node("n:alpha", "alpha"), node("n:callee", "callee")],
            vec![
                evidenced_edge("", "n:alpha", "n:callee", "first", 10),
                evidenced_edge("", "n:alpha", "n:callee", "second", 20),
                evidenced_edge("", "n:alpha", "n:callee", "third", 30),
            ],
        );
        let response = engine.discover(request(DiscoveryDirection::Outgoing))?;
        assert_eq!(response.edges.len(), 3);
        assert!(response.edges.iter().all(|edge| edge.id.is_none()));
        assert_eq!(
            response
                .edges
                .iter()
                .map(|edge| {
                    edge.occurrence_rule
                        .as_ref()
                        .map(|rule| rule.as_str().to_owned())
                })
                .collect::<Vec<_>>(),
            [
                Some("first".to_owned()),
                Some("second".to_owned()),
                Some("third".to_owned())
            ]
        );
        Ok(())
    }

    #[test]
    fn final_edge_assembly_includes_asymmetric_boundary_multigraph_evidence()
    -> Result<(), Box<dyn std::error::Error>> {
        let engine = engine(
            vec![
                node("n:alpha", "alpha"),
                node("n:left", "left"),
                node("n:right", "right"),
            ],
            vec![
                edge("e:seed-left", "n:alpha", "n:left"),
                edge("e:right-seed", "n:right", "n:alpha"),
                evidenced_edge(
                    "e:boundary:first",
                    "n:left",
                    "n:right",
                    "boundary-first",
                    10,
                ),
                evidenced_edge(
                    "e:boundary:second",
                    "n:left",
                    "n:right",
                    "boundary-second",
                    20,
                ),
            ],
        );
        let response = engine.discover(request(DiscoveryDirection::Both))?;
        assert_eq!(response.edges.len(), 4);
        let boundary = response
            .edges
            .iter()
            .filter(|edge| edge.source == "n:left" && edge.target == "n:right")
            .collect::<Vec<_>>();
        assert_eq!(boundary.len(), 2);
        assert_eq!(
            boundary
                .iter()
                .filter_map(|edge| edge.occurrence_rule.as_ref().map(|rule| rule.as_str()))
                .collect::<Vec<_>>(),
            ["boundary-first", "boundary-second"]
        );
        assert_eq!(
            boundary
                .iter()
                .filter_map(|edge| edge.relationship_site.as_ref().map(|site| site.start_line))
                .collect::<Vec<_>>(),
            [10, 20]
        );
        assert_eq!(response.omissions.edges, Some(0));
        assert_eq!(response.omissions.expanded_relationships, Some(0));
        Ok(())
    }
}
