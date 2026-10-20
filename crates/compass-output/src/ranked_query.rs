//! A bounded candidate shortlist, separate from graph traversal ledgers.
use std::collections::{BTreeMap, BTreeSet};

use compass_model::code_graph::{EdgeKind, NodeKind};
use compass_model::provenance::{
    EvidenceConfidence, EvidenceOrigin, ResolutionState, SourceAnchor,
};
use compass_model::query_contract::{
    DiscoveryEdge, DiscoveryLimits, DiscoveryScope, QueryDiagnosticCode, QueryEvidence,
    QueryEvidenceLayer, QueryNode,
};
use compass_query::{
    RankedCallLink, RankedCallWork, RankedDiscoveryResponse, RankedExcerptKind,
    RankedSourceExcerpt, RelevanceBand,
};
use serde::{Deserialize, Serialize};

use crate::{AgentCoverage, AgentEvidence, OutputError};

pub const RANKED_QUERY_VIEW_SCHEMA: &str = "compass.query.ranked-view/4";
pub const RANKED_QUERY_LINKED_VIEW_SCHEMA: &str = "compass.query.ranked-view/7";
const CODE_RANKED_QUERY_VIEW_SCHEMA: &str = "compass.query.ranked-view/3";
const COMMENT_RANKED_QUERY_VIEW_SCHEMA: &str = "compass.query.ranked-view/2";
const LEGACY_RANKED_QUERY_VIEW_SCHEMA: &str = "compass.query.ranked-view/1";
pub const RANKED_VIEW_MAX_CANDIDATES: usize = 5;
const MAX_VIEW_BYTES: usize = 128 * 1024;
const MAX_REASON_CHARS: usize = 240;
const COMPACT_REASON_CHARS: usize = 160;
const MAX_CONTEXT_EDGES: usize = 8;
// Native linked occurrences retain 64 raw proofs plus one selected-site projection.
const MAX_LINK_CONTEXT_PROOFS: usize = 65;
const MAX_CONTEXT_NODES: usize = MAX_CONTEXT_EDGES * 2;

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RankedNameMatch {
    ExactId,
    ExactName,
    Approximate,
}

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RankedRetrievalKind {
    Identity,
    Lexical,
    Signature,
    Document,
    SemanticLsa,
    CallLink,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RankedQueryCandidate {
    pub rank: u32,
    pub id: String,
    pub name: String,
    pub kind: NodeKind,
    pub source: Option<SourceAnchor>,
    pub supporting_sources: Vec<SourceAnchor>,
    pub omitted_supporting_sources: u64,
    pub relevance: RelevanceBand,
    pub name_match: RankedNameMatch,
    pub evidence: AgentEvidence,
    pub retrieval_kinds: Vec<RankedRetrievalKind>,
    pub reasons: Vec<String>,
    pub matched_terms: Vec<String>,
    pub matched_fields: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub excerpts: Vec<RankedSourceExcerpt>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub call_link: Option<RankedCallLink>,
}

/// Witnessed relationships adjacent to displayed candidates. Relevance never
/// creates a relationship; endpoint direction and occurrence evidence survive.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RankedQueryContext {
    pub nodes: Vec<QueryNode>,
    pub edges: Vec<DiscoveryEdge>,
    pub omitted_edges: u64,
    pub truncated: bool,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RankedQueryView {
    pub schema: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub call_link_work: Option<RankedCallWork>,
    pub question: String,
    pub graph_identity: String,
    pub graph_digest: String,
    pub candidates: Vec<RankedQueryCandidate>,
    pub eligible_count: Option<u64>,
    pub retained_count: u64,
    pub more_available: bool,
    pub candidate_recall_bounded: bool,
    pub response_truncated: bool,
    pub graph_coverage: AgentCoverage,
    pub scope: Vec<DiscoveryScope>,
    pub limits: DiscoveryLimits,
    pub narrowing: String,
    pub caveats: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub context: Option<RankedQueryContext>,
}

impl RankedQueryView {
    pub fn from_json(bytes: &[u8]) -> Result<Self, OutputError> {
        if bytes.len() > MAX_VIEW_BYTES {
            return Err(invalid("ranked view exceeds 128 KiB"));
        }
        #[derive(Deserialize)]
        struct Header {
            schema: String,
        }
        let header: Header = serde_json::from_slice(bytes)?;
        if !matches!(
            header.schema.as_str(),
            RANKED_QUERY_VIEW_SCHEMA
                | RANKED_QUERY_LINKED_VIEW_SCHEMA
                | CODE_RANKED_QUERY_VIEW_SCHEMA
                | COMMENT_RANKED_QUERY_VIEW_SCHEMA
                | LEGACY_RANKED_QUERY_VIEW_SCHEMA
        ) {
            return Err(invalid("unsupported ranked query view schema"));
        }
        if header.schema == LEGACY_RANKED_QUERY_VIEW_SCHEMA {
            let legacy: serde_json::Value = serde_json::from_slice(bytes)?;
            if legacy
                .get("candidates")
                .and_then(serde_json::Value::as_array)
                .is_some_and(|candidates| {
                    candidates
                        .iter()
                        .any(|candidate| candidate.get("excerpts").is_some())
                })
            {
                return Err(invalid("ranked-view/1 does not support source quotations"));
            }
        }
        let view: Self = serde_json::from_slice(bytes)?;
        view.validate()?;
        Ok(view)
    }

    fn validate(&self) -> Result<(), OutputError> {
        if !matches!(
            self.schema.as_str(),
            RANKED_QUERY_VIEW_SCHEMA
                | RANKED_QUERY_LINKED_VIEW_SCHEMA
                | CODE_RANKED_QUERY_VIEW_SCHEMA
                | COMMENT_RANKED_QUERY_VIEW_SCHEMA
                | LEGACY_RANKED_QUERY_VIEW_SCHEMA
        ) {
            return Err(invalid("unsupported ranked query view schema"));
        }
        if self.schema == RANKED_QUERY_LINKED_VIEW_SCHEMA {
            if self
                .call_link_work
                .as_ref()
                .is_none_or(|work| !work.is_coherent())
            {
                return Err(invalid(
                    "ranked-view/7 requires coherent versioned call-link work",
                ));
            }
        } else if self.call_link_work.is_some() {
            return Err(invalid("call-link work requires ranked-view/7"));
        }
        if self.graph_identity.is_empty() || self.graph_digest.is_empty() {
            return Err(invalid(
                "ranked graph identity and digest must not be empty",
            ));
        }
        if self.candidates.len() > RANKED_VIEW_MAX_CANDIDATES
            || self.retained_count < self.candidates.len() as u64
            || self
                .eligible_count
                .is_some_and(|count| count < self.retained_count)
        {
            return Err(invalid("inconsistent ranked candidate counts"));
        }
        if self.context.as_ref().is_some_and(|context| {
            context.nodes.len() > MAX_CONTEXT_NODES || context.edges.len() > MAX_CONTEXT_EDGES
        }) {
            return Err(invalid("ranked context exceeds its node or edge limit"));
        }
        let mut identities = BTreeSet::new();
        for (index, candidate) in self.candidates.iter().enumerate() {
            if candidate.id.is_empty()
                || candidate.rank as usize != index + 1
                || !identities.insert(&candidate.id)
            {
                return Err(invalid(
                    "ranked candidates must have unique IDs and consecutive ranks",
                ));
            }
            if candidate
                .source
                .iter()
                .chain(&candidate.supporting_sources)
                .any(|source| !source.is_valid())
            {
                return Err(invalid("ranked candidate source anchor is invalid"));
            }
        }
        for candidate in &self.candidates {
            if candidate.excerpts.len() > 2
                || (self.schema == LEGACY_RANKED_QUERY_VIEW_SCHEMA
                    && !candidate.excerpts.is_empty())
                || candidate.excerpts.iter().any(|quote| {
                    !quote.is_coherent()
                        || (quote.kind == RankedExcerptKind::LinkedCallablePreview
                            && (self.schema != RANKED_QUERY_LINKED_VIEW_SCHEMA
                                || candidate.call_link.is_none()
                                || !candidate.kind.is_callable()))
                        || (self.schema == COMMENT_RANKED_QUERY_VIEW_SCHEMA
                            && matches!(
                                quote.kind,
                                RankedExcerptKind::CallableCode
                                    | RankedExcerptKind::NestedCallableCode
                            ))
                        || (self.schema == CODE_RANKED_QUERY_VIEW_SCHEMA
                            && quote.kind == RankedExcerptKind::NestedCallableCode)
                        || candidate.source.as_ref().is_none_or(|source| {
                            source.file != quote.source.file
                                || match quote.kind {
                                    RankedExcerptKind::CallableComment
                                    | RankedExcerptKind::CallableCode
                                    | RankedExcerptKind::LinkedCallablePreview
                                    | RankedExcerptKind::NestedCallableCode => {
                                        quote.source.start_byte < source.start_byte
                                            || quote.source.end_byte > source.end_byte
                                            || (quote.source.start_line, quote.source.start_column)
                                                < (source.start_line, source.start_column)
                                            || (quote.source.end_line, quote.source.end_column)
                                                > (source.end_line, source.end_column)
                                            || quote.nested_scope.as_ref().is_some_and(|scope| {
                                                scope.file != source.file
                                                    || (scope.start_byte == source.start_byte
                                                        && scope.end_byte == source.end_byte)
                                                    || scope.start_byte < source.start_byte
                                                    || scope.end_byte > source.end_byte
                                                    || (scope.start_line, scope.start_column)
                                                        < (source.start_line, source.start_column)
                                                    || (scope.end_line, scope.end_column)
                                                        > (source.end_line, source.end_column)
                                            })
                                    }
                                    RankedExcerptKind::AstDocumentation => {
                                        quote.source.end_byte > source.start_byte
                                            || (quote.source.end_line, quote.source.end_column)
                                                > (source.start_line, source.start_column)
                                    }
                                }
                        })
                })
            {
                return Err(invalid(
                    "ranked source quotations exceed bounds or lack coherent source anchors",
                ));
            }
        }
        // At most five four-hop paths. Borrow metadata instead of copying it;
        // a reused compact identity must retain exactly one meaning per view.
        let mut endpoints = BTreeMap::new();
        let mut occurrences = BTreeMap::new();
        for candidate in &self.candidates {
            if let Some(link) = &candidate.call_link {
                let root = self.candidates.iter().find(|root| root.id == link.root.id);
                let terminal = link.hops.last().map(|hop| &hop.target);
                if self.schema != RANKED_QUERY_LINKED_VIEW_SCHEMA
                    || candidate.relevance != RelevanceBand::Low
                    || candidate.name_match != RankedNameMatch::Approximate
                    || !candidate
                        .retrieval_kinds
                        .contains(&RankedRetrievalKind::CallLink)
                    || !link.is_coherent_for(&candidate.id)
                    || link.hops.len() > self.limits.max_depth as usize
                    || root.is_none_or(|root| {
                        root.call_link.is_some()
                            || root.rank >= candidate.rank
                            || root.name != link.root.name
                            || root.source.as_ref() != Some(&link.root.source)
                    })
                    || terminal.is_none_or(|target| {
                        target.name != candidate.name
                            || Some(&target.source) != candidate.source.as_ref()
                    })
                {
                    return Err(invalid(
                        "call-linked inspection leads require Low relevance, displayed root, exact endpoints and ranked-view/7",
                    ));
                }
                for endpoint in
                    std::iter::once(&link.root).chain(link.hops.iter().map(|hop| &hop.target))
                {
                    if endpoints
                        .insert(endpoint.id.as_str(), endpoint)
                        .is_some_and(|old| old != endpoint)
                        || self.candidates.iter().any(|displayed| {
                            displayed.id == endpoint.id
                                && (displayed.name != endpoint.name
                                    || displayed.source.as_ref() != Some(&endpoint.source))
                        })
                        || self.context.as_ref().is_some_and(|context| {
                            context.nodes.iter().any(|displayed| {
                                displayed.id == endpoint.id
                                    && (displayed.qualified_name != endpoint.name
                                        || displayed.source.as_ref() != Some(&endpoint.source))
                            })
                        })
                    {
                        return Err(invalid(
                            "call-linked endpoint identity has contradictory metadata",
                        ));
                    }
                }
                for hop in &link.hops {
                    if occurrences
                        .insert(hop.edge_id.as_str(), hop)
                        .is_some_and(|old| old != hop)
                    {
                        return Err(invalid(
                            "call-linked occurrence identity has contradictory metadata",
                        ));
                    }
                }
            } else if candidate
                .retrieval_kinds
                .contains(&RankedRetrievalKind::CallLink)
            {
                return Err(invalid("call-link retrieval requires its witnessed path"));
            }
        }
        let required_lookups = occurrences
            .values()
            .map(|hop| hop.source_id.as_str())
            .collect::<BTreeSet<_>>()
            .len() as u64;
        if required_lookups > 0
            && self
                .call_link_work
                .as_ref()
                .is_none_or(|work| work.candidate_probes < required_lookups)
        {
            return Err(invalid(
                "call-link work under-reports its retained source lookups",
            ));
        }
        if let Some(context) = &self.context {
            if self.schema == RANKED_QUERY_LINKED_VIEW_SCHEMA {
                let mut context_occurrences = BTreeSet::new();
                for edge in &context.edges {
                    let Some(id) = edge.id.as_deref() else {
                        continue;
                    };
                    if !context_occurrences.insert(id) {
                        return Err(invalid("linked context must have unique occurrence IDs"));
                    }
                    if let Some(hop) = occurrences.get(id)
                        && (edge.source != hop.source_id
                            || edge.target != hop.target.id
                            || edge.kind != EdgeKind::Calls
                            || edge.relationship_site.as_ref() != Some(&hop.site)
                            || edge.occurrence_rule.as_ref().map(|rule| rule.as_str())
                                != hop.occurrence_rule.as_deref()
                            || edge.evidence.len() > MAX_LINK_CONTEXT_PROOFS
                            || !edge.evidence.iter().any(|proof| {
                                // A compact path retains one selected AST witness,
                                // while context may contain other proofs as well.
                                proof.layer == QueryEvidenceLayer::StructuralGraph
                                    && proof.origin == EvidenceOrigin::Ast
                                    && proof.confidence == EvidenceConfidence::Exact
                                    && proof.extractor == hop.extractor
                                    && proof.rule == hop.rule
                                    && proof.anchor.as_ref() == Some(&hop.site)
                                    && proof.resolution == ResolutionState::Exact
                                    && proof.candidates.len() <= 1
                                    && proof.candidates.iter().all(|candidate| {
                                        candidate.node_id == hop.target.id
                                            && candidate.confidence == EvidenceConfidence::Exact
                                            && candidate.reason.len() <= 1024
                                            && !candidate.reason.trim().is_empty()
                                            && candidate.score.is_none_or(f64::is_finite)
                                            && candidate.anchor.as_ref().is_none_or(|anchor| {
                                                anchor.file.len() <= 4096 && anchor.is_valid()
                                            })
                                    })
                            }))
                    {
                        return Err(invalid(
                            "compact and context occurrence identities or selected AST witnesses disagree",
                        ));
                    }
                }
            }
            let nodes = context
                .nodes
                .iter()
                .map(|node| &node.id)
                .collect::<BTreeSet<_>>();
            if nodes.len() != context.nodes.len()
                || context.nodes.iter().any(|node| {
                    node.source
                        .as_ref()
                        .is_some_and(|source| !source.is_valid())
                })
                || context.edges.iter().any(|edge| {
                    !nodes.contains(&edge.source)
                        || !nodes.contains(&edge.target)
                        || (!identities.contains(&edge.source)
                            && !identities.contains(&edge.target))
                        || edge
                            .relationship_site
                            .as_ref()
                            .is_some_and(|source| !source.is_valid())
                })
            {
                return Err(invalid(
                    "ranked context must retain witnessed endpoints adjacent to candidates",
                ));
            }
        }
        let byte_limit = MAX_VIEW_BYTES
            .min(usize::try_from(self.limits.max_response_bytes).unwrap_or(usize::MAX));
        if serde_json::to_vec(self)?.len() > byte_limit {
            return Err(invalid(
                "ranked view exceeds its response byte limit (at most 128 KiB)",
            ));
        }
        Ok(())
    }
}

pub fn build_ranked_query_view(
    response: &RankedDiscoveryResponse,
    graph_identity: &str,
    graph_digest: &str,
) -> Result<RankedQueryView, OutputError> {
    build_ranked_query_view_with_context(response, graph_identity, graph_digest, false)
}

pub fn build_ranked_query_view_with_context(
    response: &RankedDiscoveryResponse,
    graph_identity: &str,
    graph_digest: &str,
    include_context: bool,
) -> Result<RankedQueryView, OutputError> {
    if !response.discovery.has_valid_work_contract() {
        return Err(invalid(
            "unsupported discovery schema or inconsistent work counters",
        ));
    }
    if response.candidates.len() > compass_model::query_contract::MAX_DISCOVERY_NODES as usize {
        return Err(invalid(
            "ranked candidate input exceeds the discovery node limit",
        ));
    }
    let mut seen = BTreeSet::new();
    let mut candidates = Vec::new();
    for candidate in &response.candidates {
        if !candidate.score.is_finite() {
            return Err(invalid("non-finite ranked candidate score"));
        }
        if candidate.excerpts.len() > 2
            || candidate.excerpts.iter().any(|quote| !quote.is_coherent())
        {
            return Err(invalid(
                "ranked source quotations exceed bounded input limits",
            ));
        }
        if candidate.call_link.as_ref().is_some_and(|link| {
            !link.is_coherent_for(&candidate.node.id)
                || link.hops.len() > response.discovery.limits.max_depth as usize
        }) {
            return Err(invalid(
                "ranked call link exceeds bounds or has an incoherent chain",
            ));
        }
        // A declaration can be reached through several passages. Its reasons
        // are already fused by the query owner; display the owner once.
        if !seen.insert(candidate.node.id.clone()) {
            continue;
        }
        if candidates.len() == RANKED_VIEW_MAX_CANDIDATES {
            continue;
        }
        let fields = &candidate.matched_fields;
        let name_match = if fields.iter().any(|field| field == "exact_id") {
            RankedNameMatch::ExactId
        } else if fields.iter().any(|field| field == "exact_name") {
            RankedNameMatch::ExactName
        } else {
            RankedNameMatch::Approximate
        };
        let evidence = structural_evidence(&candidate.node.evidence);
        let mut retrieval_kinds = BTreeSet::new();
        for field in fields {
            retrieval_kinds.insert(match field.as_str() {
                "exact_id" | "exact_name" => RankedRetrievalKind::Identity,
                "document" | "document_link" | "owned_document" => RankedRetrievalKind::Document,
                "semantic_lsa" => RankedRetrievalKind::SemanticLsa,
                "signature" => RankedRetrievalKind::Signature,
                _ => RankedRetrievalKind::Lexical,
            });
        }
        if candidate.call_link.is_some() {
            retrieval_kinds.insert(RankedRetrievalKind::CallLink);
        }
        candidates.push(RankedQueryCandidate {
            rank: u32::try_from(candidates.len() + 1).unwrap_or(u32::MAX),
            id: candidate.node.id.clone(),
            name: candidate.node.qualified_name.clone(),
            kind: candidate.node.kind,
            source: candidate.node.source.clone(),
            supporting_sources: candidate
                .supporting_sources
                .iter()
                .take(4)
                .cloned()
                .collect(),
            omitted_supporting_sources: candidate.supporting_sources.len().saturating_sub(4) as u64,
            relevance: candidate.relevance,
            name_match,
            evidence,
            retrieval_kinds: retrieval_kinds.into_iter().collect(),
            reasons: candidate
                .reasons
                .iter()
                .take(3)
                .map(|reason| scalar(reason, MAX_REASON_CHARS))
                .collect(),
            matched_terms: candidate.matched_terms.clone(),
            matched_fields: fields.clone(),
            excerpts: candidate.excerpts.clone(),
            call_link: candidate.call_link.clone(),
        });
    }
    let retained_count = u64::try_from(seen.len()).unwrap_or(u64::MAX);
    let displayed = u64::try_from(candidates.len()).unwrap_or(u64::MAX);
    let narrowing = candidates
        .first()
        .and_then(|candidate| candidate.source.as_ref())
        .map_or_else(
            || {
                "Narrow the question with a concrete subject or add --scope source:<path>."
                    .to_owned()
            },
            |source| {
                format!(
                    "Narrow with --scope {} or name a specific behavior.",
                    shell_quote(&format!("source:{}", source.file))
                )
            },
        );
    let context = if include_context {
        if response.discovery.nodes.len()
            > compass_model::query_contract::MAX_DISCOVERY_NODES as usize
            || response.discovery.edges.len()
                > compass_model::query_contract::MAX_DISCOVERY_EDGES as usize
        {
            return Err(invalid("ranked context input exceeds discovery limits"));
        }
        let ids = candidates
            .iter()
            .map(|candidate| candidate.id.as_str())
            .collect::<BTreeSet<_>>();
        let nodes = response
            .discovery
            .nodes
            .iter()
            .map(|node| (node.id.as_str(), node))
            .collect::<BTreeMap<_, _>>();
        let adjacent = response
            .discovery
            .edges
            .iter()
            .filter(|edge| ids.contains(edge.source.as_str()) || ids.contains(edge.target.as_str()))
            .collect::<Vec<_>>();
        let edges = adjacent
            .iter()
            .take(MAX_CONTEXT_EDGES)
            .map(|edge| (*edge).clone())
            .collect::<Vec<_>>();
        let mut endpoints = BTreeMap::new();
        for edge in &edges {
            for id in [&edge.source, &edge.target] {
                let node = nodes
                    .get(id.as_str())
                    .ok_or_else(|| invalid("ranked context relationship lacks an endpoint"))?;
                endpoints.insert(id.clone(), (*node).clone());
            }
        }
        let omitted_edges = adjacent.len().saturating_sub(edges.len()) as u64;
        Some(RankedQueryContext {
            nodes: endpoints.into_values().collect(),
            edges,
            omitted_edges,
            truncated: response.discovery.truncated || omitted_edges > 0,
        })
    } else {
        None
    };
    let view = RankedQueryView {
        schema: if response.discovery.stats.call_link_work.is_some() {
            RANKED_QUERY_LINKED_VIEW_SCHEMA
        } else {
            RANKED_QUERY_VIEW_SCHEMA
        }
        .to_owned(),
        call_link_work: response.discovery.stats.call_link_work.clone(),
        question: response.discovery.question.clone(),
        graph_identity: graph_identity.to_owned(),
        graph_digest: graph_digest.to_owned(),
        candidates,
        eligible_count: response.eligible_count,
        retained_count,
        more_available: response.bounded
            || retained_count > displayed
            || response
                .eligible_count
                .is_some_and(|count| count > displayed),
        candidate_recall_bounded: response.bounded,
        response_truncated: response.discovery.truncated,
        graph_coverage: if response
            .discovery
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.code == QueryDiagnosticCode::IncompleteCoverage)
        {
            AgentCoverage::Incomplete
        } else {
            AgentCoverage::Unknown
        },
        scope: response.discovery.scope.clone(),
        limits: response.discovery.limits.clone(),
        narrowing,
        caveats: response
            .discovery
            .diagnostics
            .iter()
            .take(8)
            .map(|diagnostic| scalar(&diagnostic.message, MAX_REASON_CHARS))
            .collect(),
        context,
    };
    view.validate()?;
    Ok(view)
}

/// Try abbreviated reasons before dropping whole candidate rows, reserving
/// uncertainty, metadata and refinement in the same UTF-8 byte budget.
/// No cursor contract is reused.
pub fn render_ranked_query_text(
    view: &RankedQueryView,
    token_budget: usize,
) -> Result<String, OutputError> {
    view.validate()?;
    let limit = token_budget.saturating_mul(4).saturating_sub(1);
    let total = view
        .eligible_count
        .map_or_else(|| "unknown".to_owned(), |count| count.to_string());
    for shown in (0..=view.candidates.len()).rev() {
        if shown == 0 && !view.candidates.is_empty() {
            break;
        }
        // Preserve the ranked shortlist before preserving lengthy prose.
        // JSON retains the original bounded reasons; text makes elision explicit.
        for (compact_reasons, first_excerpt_chars, other_excerpt_chars) in [
            (false, 180, 180),
            (true, 180, 180),
            (true, 180, 80),
            (true, 180, 0),
            (true, 80, 80),
            (true, 0, 0),
        ] {
            let mut text = if view.candidates.is_empty() {
                "No supporting indexed evidence found. This does not prove the behavior is absent.\n"
                .to_owned()
            } else {
                format!("Ranked candidates: {shown} shown; {total} eligible.\n")
            };
            if let Some(work) = &view.call_link_work {
                text.push_str(&format!(
                    "Call-linked lookups: {}/{}; indexed recall probes: {}/{}.\n",
                    work.candidate_probes,
                    work.candidate_probe_limit,
                    work.indexed_candidate_probes,
                    work.indexed_candidate_probe_limit
                ));
            }
            for candidate in view.candidates.iter().take(shown) {
                let excerpt_chars = if candidate.rank == 1 {
                    first_excerpt_chars
                } else {
                    other_excerpt_chars
                };
                let source = candidate.source.as_ref().map_or_else(
                    || "source unavailable".to_owned(),
                    |source| format!("{}:{}", scalar(&source.file, 4096), source.start_line),
                );
                let reasons = candidate
                    .reasons
                    .iter()
                    .map(|reason| scalar(reason, MAX_REASON_CHARS))
                    .collect::<Vec<_>>()
                    .join("; ");
                let reasons = if compact_reasons {
                    scalar(&reasons, COMPACT_REASON_CHARS)
                } else {
                    reasons
                };
                text.push_str(&format!(
                    "{}. {} @ {} [relevance={}; name={}; evidence={}; via={}; kind={}]\n   {}\n",
                    candidate.rank,
                    scalar(&candidate.name, 512),
                    source,
                    band(candidate.relevance),
                    match_name(candidate.name_match),
                    evidence_name(candidate.evidence),
                    candidate
                        .retrieval_kinds
                        .iter()
                        .map(|kind| retrieval_name(*kind))
                        .collect::<Vec<_>>()
                        .join(","),
                    candidate.kind.as_str(),
                    reasons
                ));
                if let Some(link) = &candidate.call_link {
                    let names = std::iter::once(&link.root)
                        .chain(link.hops.iter().map(|hop| &hop.target))
                        .map(|endpoint| {
                            scalar(
                                endpoint
                                    .name
                                    .rsplit([':', '.'])
                                    .find(|part| !part.is_empty())
                                    .unwrap_or(&endpoint.name),
                                64,
                            )
                        })
                        .collect::<Vec<_>>()
                        .join(" → ");
                    text.push_str(&format!(
                        "   Call path (execution unverified): {names}; navigation hint {} → {}\n",
                        scalar(&link.hint.requested, 128),
                        scalar(&link.hint.spelling, 128)
                    ));
                }
                for quote in candidate
                    .excerpts
                    .iter()
                    .take(usize::from(excerpt_chars > 0))
                {
                    text.push_str(&format!(
                        "   {} @ {}:{}: \"{}\"{}\n",
                        match quote.kind {
                            RankedExcerptKind::CallableComment => "Comment",
                            RankedExcerptKind::CallableCode => "Code",
                            RankedExcerptKind::LinkedCallablePreview =>
                                "Helper code preview (behavior unverified)",
                            RankedExcerptKind::NestedCallableCode =>
                                "Nested lexical code (execution unverified)",
                            RankedExcerptKind::AstDocumentation => "Documentation",
                        },
                        scalar(&quote.source.file, 4096),
                        quote.source.start_line,
                        quote_scalar(&quote.text, excerpt_chars),
                        if quote.clipped || quote.text.chars().count() > excerpt_chars {
                            " [excerpt clipped]"
                        } else {
                            ""
                        }
                    ));
                }
                if let Some(source) = candidate.supporting_sources.first() {
                    text.push_str(&format!(
                        "   supporting passage: {}:{}\n",
                        scalar(&source.file, 4096),
                        source.start_line,
                    ));
                }
            }
            if view
                .candidates
                .iter()
                .take(shown)
                .any(|candidate| !candidate.excerpts.is_empty())
            {
                text.push_str(if first_excerpt_chars == 0 {
                    "Quotes omitted for budget; see --format json.\n"
                } else {
                    "Comments/documentation are inspection evidence; behavior remains unverified.\n"
                });
                if first_excerpt_chars > 0
                    && other_excerpt_chars == 0
                    && view
                        .candidates
                        .iter()
                        .take(shown)
                        .skip(1)
                        .any(|candidate| !candidate.excerpts.is_empty())
                {
                    text.push_str("Lower-ranked quotes omitted for budget; see --format json.\n");
                }
            }
            if compact_reasons && shown > 0 {
                text.push_str("Reasons abbreviated; --format json shows more detail.\n");
                if view
                    .candidates
                    .iter()
                    .take(shown)
                    .any(|candidate| candidate.relevance == RelevanceBand::Low)
                {
                    text.push_str(
                        "Low matches are inspection leads; the requested behavior is unverified.\n",
                    );
                }
            }
            text.push_str(&format!(
                "Coverage: {}; candidate recall {}; response {}.\n",
                if view.graph_coverage == AgentCoverage::Incomplete {
                    "incomplete"
                } else {
                    "unknown"
                },
                if view.candidate_recall_bounded {
                    "bounded"
                } else {
                    "complete within indexed scope"
                },
                if view.response_truncated {
                    "truncated"
                } else {
                    "within limits"
                }
            ));
            if view.candidate_recall_bounded {
                text.push_str(
                    "Additional candidates may exist outside these bounds; refine the scope.\n",
                );
            } else if view.more_available || shown < view.candidates.len() {
                text.push_str("More available; refine the scope to inspect other candidates.\n");
            }
            if let Some(caveat) = view.caveats.first() {
                text.push_str(&format!("Note: {}\n", scalar(caveat, MAX_REASON_CHARS)));
            }
            if let Some(context) = &view.context {
                let shown_ids = view
                    .candidates
                    .iter()
                    .take(shown)
                    .map(|candidate| candidate.id.as_str())
                    .collect::<BTreeSet<_>>();
                let edges = context
                    .edges
                    .iter()
                    .filter(|edge| {
                        shown_ids.contains(edge.source.as_str())
                            || shown_ids.contains(edge.target.as_str())
                    })
                    .collect::<Vec<_>>();
                for edge_count in (0..=edges.len()).rev() {
                    let mut with_context = text.clone();
                    let omitted = context
                        .omitted_edges
                        .saturating_add(context.edges.len().saturating_sub(edge_count) as u64);
                    with_context.push_str(&format!(
                        "Context (one hop): {edge_count} edges shown; {omitted} omitted{}.\n",
                        if context.truncated { "; bounded" } else { "" }
                    ));
                    for edge in edges.iter().take(edge_count) {
                        let name = |id: &str| {
                            context.nodes.iter().find(|node| node.id == id).map_or_else(
                                || scalar(id, 256),
                                |node| scalar(&node.qualified_name, 256),
                            )
                        };
                        let source = edge.relationship_site.as_ref().map_or_else(
                            || "source unavailable".to_owned(),
                            |source| {
                                format!("{}:{}", scalar(&source.file, 4096), source.start_line)
                            },
                        );
                        with_context.push_str(&format!(
                            "  {} --{}--> {} @ {} [evidence={}]\n",
                            name(&edge.source),
                            edge.kind.as_str(),
                            name(&edge.target),
                            source,
                            evidence_name(structural_evidence(&edge.evidence))
                        ));
                    }
                    with_context.push_str(&scalar(&view.narrowing, 4096));
                    if with_context.len() <= limit {
                        return Ok(with_context);
                    }
                }
            } else {
                text.push_str(&scalar(&view.narrowing, 4096));
                if text.len() <= limit {
                    return Ok(text);
                }
            }
        }
    }
    Err(invalid(
        "ranked answer metadata and one candidate exceed --text-budget; increase the budget",
    ))
}

fn structural_evidence(evidence: &[QueryEvidence]) -> AgentEvidence {
    use compass_model::provenance::{EvidenceConfidence, ResolutionState};
    if evidence.iter().any(|item| {
        item.confidence == EvidenceConfidence::Ambiguous
            || item.resolution == ResolutionState::Ambiguous
    }) {
        return AgentEvidence::Ambiguous;
    }
    let exact = evidence
        .iter()
        .any(|item| item.confidence == EvidenceConfidence::Exact);
    let inferred = evidence
        .iter()
        .any(|item| item.confidence != EvidenceConfidence::Exact);
    match (exact, inferred) {
        (true, true) => AgentEvidence::Mixed,
        (true, false) => AgentEvidence::Exact,
        (false, true) => AgentEvidence::Inferred,
        (false, false) => AgentEvidence::None,
    }
}

fn invalid(message: &str) -> OutputError {
    OutputError::InvalidAgentQuery(message.to_owned())
}
fn quote_scalar(value: &str, chars: usize) -> String {
    value
        .chars()
        .take(chars)
        .flat_map(|character| {
            if character.is_control()
                || matches!(character, '\u{202a}'..='\u{202e}' | '\u{2066}'..='\u{2069}')
            {
                character.escape_unicode().collect::<Vec<_>>()
            } else if matches!(character, '"' | '\\') {
                character.escape_default().collect::<Vec<_>>()
            } else {
                vec![character]
            }
        })
        .collect()
}
fn scalar(value: &str, chars: usize) -> String {
    value
        .chars()
        .take(chars)
        .flat_map(|character| {
            if character.is_control() {
                character.escape_default().collect::<Vec<_>>()
            } else {
                vec![character]
            }
        })
        .collect()
}
fn shell_quote(value: &str) -> String {
    format!("'{}'", scalar(value, 4096).replace('\'', "'\"'\"'"))
}
fn band(value: RelevanceBand) -> &'static str {
    match value {
        RelevanceBand::High => "high",
        RelevanceBand::Medium => "medium",
        RelevanceBand::Low => "low",
    }
}
fn evidence_name(value: AgentEvidence) -> &'static str {
    match value {
        AgentEvidence::Exact => "exact",
        AgentEvidence::Inferred => "inferred",
        AgentEvidence::Mixed => "mixed",
        AgentEvidence::Ambiguous => "ambiguous",
        AgentEvidence::None => "none",
    }
}

fn match_name(value: RankedNameMatch) -> &'static str {
    match value {
        RankedNameMatch::ExactId => "exact_id",
        RankedNameMatch::ExactName => "exact_name",
        RankedNameMatch::Approximate => "approximate",
    }
}

fn retrieval_name(value: RankedRetrievalKind) -> &'static str {
    match value {
        RankedRetrievalKind::Identity => "identity",
        RankedRetrievalKind::Lexical => "lexical",
        RankedRetrievalKind::Signature => "signature",
        RankedRetrievalKind::Document => "document",
        RankedRetrievalKind::SemanticLsa => "semantic_lsa",
        RankedRetrievalKind::CallLink => "call_link",
    }
}
