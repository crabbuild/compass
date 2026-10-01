//! Bounded approximate concept recall over the published native graph.
use std::collections::{BTreeMap, BTreeSet, VecDeque};

use crate::QueryError;
use crate::code_query::{CodeQueryEngine, normalize_symbol, query_edge, query_node};
use crate::telemetry::QueryInstrumentation;
use compass_model::code_graph::{EdgeKind, NodeDetails, NodeKind, ResourceKind};
use compass_model::query_contract::{
    CodeQueryResponse, ConceptMatch, ConceptMatchMethod, QueryDiagnostic, QueryDiagnosticCode,
    QueryNode, SearchHit, SearchRequest,
};

const MAX_EXPANSIONS: usize = 12;
const MAX_CONCEPT_CANDIDATES: u32 = 64;
const MAX_CONCEPT_POSTINGS: usize = 768;
const MAX_DOCUMENT_ROOTS: usize = 8;
const MAX_DOCUMENT_PROBES: usize = 64;

// These are recall alternatives, never assertions of equivalent code meaning.
const SYNONYMS: &[(&[&str], &[&str])] = &[
    (
        &["authorization", "access control", "permissions"],
        &["authz", "permission", "policy", "access", "auth"],
    ),
    (
        &["authentication", "sign in", "log in"],
        &["authn", "login", "credential", "oauth", "token"],
    ),
    (
        &["configuration", "config", "environment variables"],
        &["settings", "env", "config"],
    ),
    (
        &["persistence", "data storage", "database access"],
        &["repository", "dao", "database", "store", "db"],
    ),
    (
        &["validation", "input checking"],
        &["validate", "validator", "validation", "schema"],
    ),
    (
        &["rate limiting", "rate limit", "request throttling"],
        &["rate limit", "ratelimit", "limiter", "throttle", "quota"],
    ),
    (
        &[
            "background jobs",
            "background work",
            "job scheduling",
            "recurring jobs",
        ],
        &["scheduler", "schedule", "worker", "queue", "task"],
    ),
    (
        &["caching", "cache invalidation"],
        &["cache", "evict", "ttl"],
    ),
    (
        &["database transactions", "unit of work"],
        &["transaction", "uow", "session"],
    ),
    (
        &["feature flags", "feature toggles"],
        &["feature", "flag", "toggle", "launchdarkly"],
    ),
    (
        &["error handling", "exceptions"],
        &["error", "exception", "handler"],
    ),
    (
        &["observability", "monitoring"],
        &["telemetry", "metrics", "tracing", "logging"],
    ),
    (&["logging", "audit trail"], &["logger", "log", "audit"]),
    (
        &["concurrency", "synchronization", "simultaneous"],
        &["mutex", "lock", "semaphore"],
    ),
    (
        &["serialization", "encoding"],
        &["serialize", "serializer", "codec", "encode"],
    ),
    (
        &[
            "retry",
            "retries",
            "transient failures",
            "transient database failures",
        ],
        &["backoff", "retry", "attempt"],
    ),
    (&["timeouts", "request timeout"], &["timeout", "deadline"]),
    (
        &["encryption", "cryptography", "encrypted"],
        &["encrypt", "crypto", "cipher", "encrypter", "rsa"],
    ),
    (
        &["file uploads", "file upload"],
        &["upload", "multipart", "attachment", "file"],
    ),
    (
        &["email delivery", "sending email"],
        &["smtp", "mail", "email"],
    ),
    (
        &["pagination", "result paging"],
        &["pagination", "paginate", "cursor", "page"],
    ),
    (
        &["notifications", "event delivery"],
        &["notify", "notification", "event"],
    ),
    (
        &["subscription payments", "billing", "payments"],
        &["billing", "invoice", "payment", "stripe", "subscription"],
    ),
    (
        &["deployment", "container orchestration"],
        &["deploy", "kubernetes", "k8s", "docker"],
    ),
    (
        &["routing", "http endpoints"],
        &["router", "route", "controller", "endpoint"],
    ),
    (
        &["dependency injection"],
        &["inject", "provider", "container"],
    ),
    (
        &["search indexing", "full text search"],
        &["index", "fts", "search"],
    ),
    (
        &["database migrations", "schema changes"],
        &["migration", "alembic", "revision"],
    ),
    (
        &["health checks", "service health"],
        &["health", "healthcheck", "readiness", "liveness"],
    ),
    (
        &["data deletion", "retention policy", "data retention"],
        &["retention", "delete", "purge", "gc"],
    ),
];

pub(crate) fn expansions(question: &str) -> Vec<(String, String)> {
    // Match whole words/phrases, not identifier substrings (authorship != auth).
    let words = question
        .to_ascii_lowercase()
        .split(|c: char| !c.is_alphanumeric())
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>()
        .join(" ");
    let padded = format!(" {words} ");
    let mut result = BTreeSet::new();
    for (phrases, alternatives) in SYNONYMS {
        if let Some(phrase) = phrases
            .iter()
            .find(|phrase| padded.contains(&format!(" {phrase} ")))
        {
            for alternative in *alternatives {
                result.insert(((*phrase).to_owned(), (*alternative).to_owned()));
            }
        }
    }
    result.into_iter().take(MAX_EXPANSIONS).collect()
}

fn document_content(node: &QueryNode) -> Option<&str> {
    match &node.details {
        Some(NodeDetails::Document(details)) => details.content.as_deref(),
        Some(NodeDetails::Resource(details)) => details.content.as_deref(),
        _ => None,
    }
}

fn is_document(node: &QueryNode) -> bool {
    matches!(&node.details, Some(NodeDetails::Document(_)))
        || matches!(&node.details, Some(NodeDetails::Resource(details)) if matches!(details.resource_kind, ResourceKind::Document | ResourceKind::Rationale))
}

fn terms(value: &str) -> BTreeSet<String> {
    compass_model::search::identifier_search_terms(value)
        .into_iter()
        .map(compass_model::canonical_code_token)
        .collect()
}

fn name_matches(node: &QueryNode, query: &[String]) -> bool {
    let mut node_terms = terms(&node.name);
    node_terms.extend(terms(&node.qualified_name));
    if let Some(source) = &node.source {
        node_terms.extend(terms(&source.file));
    }
    query
        .iter()
        .all(|term| node_terms.contains(&compass_model::canonical_code_token(term.clone())))
}

fn implementation_candidate(node: &compass_model::code_graph::NodeRecord) -> bool {
    !matches!(
        node.kind,
        NodeKind::Parameter
            | NodeKind::Property
            | NodeKind::Field
            | NodeKind::Variable
            | NodeKind::Constant
            | NodeKind::Import
            | NodeKind::Export
    )
}

fn kind_signal(node: &QueryNode) -> f64 {
    match node.kind {
        NodeKind::Class
        | NodeKind::Struct
        | NodeKind::Interface
        | NodeKind::Trait
        | NodeKind::Protocol => 250_000.0,
        NodeKind::Function | NodeKind::Method | NodeKind::Constructor => 200_000.0,
        NodeKind::Module | NodeKind::Package | NodeKind::Namespace | NodeKind::File => 150_000.0,
        NodeKind::Enum | NodeKind::TypeAlias => 100_000.0,
        NodeKind::Resource => -300_000.0,
        _ => -600_000.0,
    }
}

fn context_terms(question_terms: &[String], phrase: &str) -> BTreeSet<String> {
    let phrase_terms = terms(phrase);
    question_terms
        .iter()
        .filter(|term| {
            (!phrase_terms.contains(*term)
                || matches!(
                    term.as_str(),
                    "database" | "file" | "request" | "workspace" | "application"
                ))
                && !matches!(
                    term.as_str(),
                    "handle"
                        | "handles"
                        | "handled"
                        | "check"
                        | "checks"
                        | "checked"
                        | "enforce"
                        | "enforced"
                        | "load"
                        | "loaded"
                        | "read"
                        | "configure"
                        | "configured"
                        | "run"
                        | "implement"
                        | "prevents"
                        | "stored"
                        | "result"
                        | "user"
                )
        })
        .map(|term| match term.as_str() {
            "database" => "db".into(),
            "uploaded" => "upload".into(),
            "application" => "app".into(),
            _ => term.clone(),
        })
        .collect()
}

fn configuration_intent(question_terms: &[String]) -> bool {
    question_terms.iter().any(|term| {
        matches!(
            term.as_str(),
            "configure" | "configuration" | "setting" | "config" | "env"
        )
    })
}

fn synonym_score(
    node: &QueryNode,
    phrase: &str,
    alternative: &str,
    question_terms: &[String],
) -> f64 {
    let synonyms = SYNONYMS
        .iter()
        .find(|(phrases, _)| phrases.contains(&phrase))
        .map(|(_, alternatives)| *alternatives)
        .unwrap_or_default();
    let priority = synonyms
        .iter()
        .position(|candidate| *candidate == alternative)
        .map_or(0, |index| synonyms.len().saturating_sub(index));
    let alternative_terms = crate::search_tokens(alternative);
    let names = terms(&node.name);
    let qualified = terms(&node.qualified_name);
    let path = node
        .source
        .as_ref()
        .map_or("", |source| source.file.as_str());
    let source_terms = terms(path);
    let direct = alternative_terms.iter().all(|term| names.contains(term));
    let owner = alternative_terms
        .iter()
        .all(|term| qualified.contains(term));
    let file = path.rsplit('/').next().unwrap_or_default();
    let file_terms = terms(file);
    let file_words =
        compass_model::identifier_tokens(file.rsplit_once('.').map_or(file, |(stem, _)| stem));
    let file_match = alternative_terms
        .iter()
        .all(|term| file_terms.contains(term));
    let mut all = names.clone();
    all.extend(qualified);
    all.extend(source_terms);
    if all.contains("database") {
        all.insert("db".into());
    }
    let context_count = context_terms(question_terms, phrase)
        .iter()
        .filter(|term| all.contains(*term))
        .take(4)
        .count();
    let test = path
        .split('/')
        .any(|part| matches!(part, "tests" | "test" | "fixtures" | "generated"));
    let data_path = path
        .split('/')
        .any(|part| matches!(part, "models" | "entities" | "contracts" | "fields"));
    let data_signature = matches!(&node.details, Some(NodeDetails::Symbol(details)) if details.signature.as_deref().is_some_and(|signature| signature.contains("NamedTuple") || signature.contains("TypedDict")));
    let utility = path.split('/').count() <= 2
        && path
            .split('/')
            .next()
            .is_some_and(|part| matches!(part, "libs" | "lib" | "utils" | "extensions"));
    let config_source = path
        .split('/')
        .any(|part| matches!(part, "configs" | "config" | "settings" | "extensions"));
    let data_type = matches!(
        node.kind,
        NodeKind::Class | NodeKind::Struct | NodeKind::Interface | NodeKind::TypeAlias
    ) && (data_path
        || data_signature
        || names.iter().any(|term| {
            matches!(
                term.as_str(),
                "error"
                    | "exception"
                    | "response"
                    | "request"
                    | "payload"
                    | "params"
                    | "entity"
                    | "record"
                    | "contract"
            )
        }));
    // Compact identifiers and a concept-bearing filename favor a reusable
    // implementation over incidental methods and data-transfer declarations.
    2_000_000.0
        + kind_signal(node)
        + priority as f64 * 25_000.0
        + if direct {
            300_000.0
        } else if owner {
            150_000.0
        } else {
            0.0
        }
        + if file_match {
            350_000.0 * alternative_terms.len() as f64
                / file_words.len().max(alternative_terms.len()).max(1) as f64
        } else {
            0.0
        }
        + if utility { 300_000.0 } else { 0.0 }
        + if configuration_intent(question_terms) && config_source {
            800_000.0
        } else {
            0.0
        }
        + context_count as f64 * 300_000.0
        + 200_000.0 / names.len().max(1) as f64
        - path.split('/').count() as f64 * 20_000.0
        - if data_type { 500_000.0 } else { 0.0 }
        - if test || node.source.is_none() {
            500_000.0
        } else {
            0.0
        }
}

impl CodeQueryEngine {
    pub(crate) fn search_instrumented(
        &self,
        request: SearchRequest,
        instrumentation: &mut QueryInstrumentation,
    ) -> Result<CodeQueryResponse, QueryError> {
        self.search_concepts_instrumented(request, instrumentation, &|_| true)
    }

    pub(crate) fn search_concepts_instrumented(
        &self,
        request: SearchRequest,
        instrumentation: &mut QueryInstrumentation,
        admit: &dyn Fn(&compass_model::code_graph::NodeRecord) -> bool,
    ) -> Result<CodeQueryResponse, QueryError> {
        let mut response = self.search_lexical_instrumented(request.clone(), instrumentation)?;
        // Scoped discovery applies admission before concept ranking/capping.
        // Keep document roots available so an in-scope target can still be
        // reached from an out-of-scope design note.
        let mut admitted_ids = BTreeSet::new();
        for node in &response.nodes {
            if is_document(node)
                || self
                    .backend
                    .node_by_id(&node.id)?
                    .is_some_and(|record| admit(&record))
            {
                admitted_ids.insert(node.id.clone());
            }
        }
        let normalized = normalize_symbol(&request.query);
        if response.nodes.iter().any(|node| {
            admitted_ids.contains(&node.id)
                && !is_document(node)
                && (node.id == request.query
                    || normalize_symbol(&node.name) == normalized
                    || normalize_symbol(&node.qualified_name) == normalized)
        }) {
            return Ok(response);
        }
        response
            .results
            .retain(|hit| admitted_ids.contains(&hit.node_id));
        let mut original_terms = crate::query_terms(&request.query);
        // Preserve acronym spellings (OAuth -> oauth) alongside subwords.
        for word in request.query.split(|c: char| !c.is_alphanumeric()) {
            if word.chars().filter(|c| c.is_uppercase()).count() >= 2 && word.len() >= 3 {
                original_terms.push(compass_model::canonical_code_token(word.to_lowercase()));
            }
        }
        original_terms.sort();
        original_terms.dedup();
        let mut nodes = response
            .nodes
            .iter()
            .cloned()
            .map(|node| (node.id.clone(), node))
            .collect::<BTreeMap<_, _>>();
        let mut hits = response
            .results
            .iter()
            .cloned()
            .map(|hit| (hit.node_id.clone(), hit))
            .collect::<BTreeMap<_, _>>();
        let mut matches = Vec::new();
        let variants = expansions(&request.query);
        for (concept, alternative) in &variants {
            self.check_deadline()?;
            // Read a bounded posting envelope before ranking. Truncating an
            // ID-ordered posting list to 64 first makes important declarations
            // disappear behind fields, parameters and data-transfer types.
            let remaining = compass_model::query_contract::MAX_INDEXED_CANDIDATE_NODES_READ
                .saturating_sub(instrumentation.work.candidates_read);
            let terms = crate::search_tokens(alternative);
            let posting_limit = MAX_CONCEPT_POSTINGS.min(remaining as usize / terms.len().max(1));
            if posting_limit < compass_graph::GRAPH_TERM_POSTING_CHUNK_ITEMS {
                response.truncated = true;
                break;
            }
            let fts = terms
                .iter()
                .map(|term| format!("\"{}\"*", term.replace('"', "\"\"")))
                .collect::<Vec<_>>()
                .join(" AND ");
            let (recalled, limited, decoded) = if let Some(read) = self
                .backend
                .store_term_candidates(&terms, posting_limit, true)?
            {
                (read.nodes, read.truncated, read.node_ids_decoded)
            } else {
                let (nodes, limited) = self.materialized_term_candidates(&fts, posting_limit)?;
                let decoded = nodes.len() as u64;
                (nodes, limited, decoded)
            };
            response.truncated |= limited;
            instrumentation.work.candidates_read =
                instrumentation.work.candidates_read.saturating_add(decoded);
            instrumentation.work.postings_decoded = instrumentation
                .work
                .postings_decoded
                .saturating_add(decoded);
            let mut ranked = recalled
                .iter()
                .filter(|node| {
                    admit(node)
                        && (implementation_candidate(node)
                            || (configuration_intent(&original_terms)
                                && matches!(node.kind, NodeKind::Field | NodeKind::ConfigKey)
                                && node.source.as_ref().is_some_and(|source| {
                                    source.file.split('/').any(|part| {
                                        matches!(part, "configs" | "config" | "settings")
                                    })
                                })))
                })
                .map(query_node)
                .filter(|node| name_matches(node, &terms))
                .map(|node| {
                    let score = synonym_score(&node, concept, alternative, &original_terms);
                    (node, score)
                })
                .collect::<Vec<_>>();
            ranked.sort_by(|a, b| b.1.total_cmp(&a.1).then_with(|| a.0.id.cmp(&b.0.id)));
            let cap = request.limits.max_candidates.min(MAX_CONCEPT_CANDIDATES) as usize;
            response.truncated |= ranked.len() > cap;
            for (node, score) in ranked.into_iter().take(cap) {
                nodes.insert(node.id.clone(), node.clone());
                let candidate = hits.entry(node.id.clone()).or_insert(SearchHit {
                    node_id: node.id.clone(),
                    score,
                    matched_fields: vec!["synonym".to_owned()],
                });
                candidate.score = candidate.score.max(score);
                matches.push(ConceptMatch {
                    node_id: node.id.clone(),
                    method: ConceptMatchMethod::Synonym,
                    query: concept.clone(),
                    matched: alternative.clone(),
                    via: Vec::new(),
                });
            }
        }
        let has_name_match = nodes.values().any(|node| {
            hits.contains_key(&node.id) && !is_document(node) && name_matches(node, &original_terms)
        });
        if self.semantic_search && !has_name_match && matches.is_empty() {
            for (node, similarity) in
                self.semantic_candidates(&request.query, request.limits.max_nodes)?
            {
                if !admit(&node) {
                    continue;
                }
                let node = query_node(&node);
                hits.insert(
                    node.id.clone(),
                    SearchHit {
                        node_id: node.id.clone(),
                        score: 1_000_000.0 + similarity * 1000.0,
                        matched_fields: vec!["semantic_lsa".to_owned()],
                    },
                );
                matches.push(ConceptMatch {
                    node_id: node.id.clone(),
                    method: ConceptMatchMethod::SemanticLsa,
                    query: request.query.clone(),
                    matched: "local-lsa/1 (corpus co-occurrence; approximate)".to_owned(),
                    via: Vec::new(),
                });
                nodes.insert(node.id.clone(), node);
            }
        }
        // Follow only witnessed document relationships. Containment reaches a
        // paragraph from a matching heading; it never fabricates a code edge.
        let mut documents = nodes
            .values()
            .filter(|node| {
                if is_document(node) {
                    let text = document_content(node)
                        .unwrap_or_default()
                        .chars()
                        .take(8192)
                        .collect::<String>();
                    let terms = crate::query_terms(&text);
                    name_matches(node, &original_terms)
                        || original_terms.iter().any(|term| terms.contains(term))
                        || matches.iter().any(|matched| matched.node_id == node.id)
                } else {
                    false
                }
            })
            .map(|node| node.id.clone())
            .collect::<Vec<_>>();
        documents.sort_by(|left, right| {
            hits[right]
                .score
                .total_cmp(&hits[left].score)
                .then_with(|| left.cmp(right))
        });
        if documents.len() > MAX_DOCUMENT_ROOTS {
            response.truncated = true;
        }
        let mut pending = documents
            .into_iter()
            .take(MAX_DOCUMENT_ROOTS)
            .map(|id| (id.clone(), vec![id], 0_u32))
            .collect::<VecDeque<_>>();
        let mut visited = BTreeSet::new();
        let mut edges = BTreeMap::new();
        let mut expanded_edges = 0_usize;
        while let Some((owner, via, depth)) = pending.pop_front() {
            self.check_deadline()?;
            if !visited.insert(owner.clone()) {
                continue;
            }
            if visited.len() > MAX_DOCUMENT_PROBES {
                response.truncated = true;
                break;
            }
            let remaining = usize::try_from(request.limits.max_edges)
                .unwrap_or(usize::MAX)
                .saturating_sub(expanded_edges);
            if remaining == 0 {
                response.truncated = true;
                break;
            }
            let (outgoing, limited) = self.backend.matching_bounded(
                &owner,
                false,
                &[
                    EdgeKind::Documents,
                    EdgeKind::References,
                    EdgeKind::Contains,
                ],
                false,
                remaining,
            )?;
            response.truncated |= limited;
            expanded_edges = expanded_edges.saturating_add(outgoing.len());
            instrumentation.work.edges_expanded +=
                u64::try_from(outgoing.len()).unwrap_or(u64::MAX);
            for edge in outgoing {
                let Some(target) = self.backend.node_by_id(&edge.target)? else {
                    continue;
                };
                let admitted = admit(&target);
                let target = query_node(&target);
                let target_is_document = is_document(&target);
                if target_is_document && edge.kind == EdgeKind::Contains {
                    if depth >= request.limits.max_depth.min(2) {
                        response.truncated = true;
                        continue;
                    }
                    nodes.insert(target.id.clone(), target.clone());
                    edges.insert(edge.id.clone(), query_edge(&edge));
                    let mut path = via.clone();
                    path.push(target.id.clone());
                    pending.push_back((target.id, path, depth + 1));
                } else if admitted
                    && !target_is_document
                    && edge.kind != EdgeKind::Contains
                    && target.kind != NodeKind::Resource
                {
                    nodes.insert(target.id.clone(), target.clone());
                    let score = hits.get(&via[0]).map_or(0.0, |hit| hit.score) + 100_000.0;
                    hits.entry(target.id.clone()).or_insert(SearchHit {
                        node_id: target.id.clone(),
                        score,
                        matched_fields: vec!["document_link".to_owned()],
                    });
                    edges.insert(edge.id.clone(), query_edge(&edge));
                    matches.push(ConceptMatch {
                        node_id: target.id,
                        method: ConceptMatchMethod::DocumentLink,
                        query: request.query.clone(),
                        matched: target.qualified_name,
                        via: via.clone(),
                    });
                }
            }
        }
        if matches.is_empty() {
            return Ok(response);
        }
        response.results = hits.into_values().collect();
        response.results.sort_by(|a, b| {
            b.score
                .total_cmp(&a.score)
                .then_with(|| a.node_id.cmp(&b.node_id))
        });
        let cap = usize::try_from(request.limits.max_nodes).unwrap_or(usize::MAX);
        response.truncated |= response.results.len() > cap;
        response.results.truncate(cap);
        let mut retained = response
            .results
            .iter()
            .map(|hit| hit.node_id.clone())
            .collect::<BTreeSet<_>>();
        // Retain the provenance path only when it fits, keeping endpoints coherent.
        for item in &matches {
            if retained.contains(&item.node_id) {
                for id in &item.via {
                    if retained.len() < cap {
                        retained.insert(id.clone());
                    } else if !retained.contains(id) {
                        response.truncated = true;
                    }
                }
            }
        }
        response.nodes = nodes
            .into_values()
            .filter(|node| retained.contains(&node.id))
            .collect();
        response.edges = edges
            .into_values()
            .filter(|edge| retained.contains(&edge.source) && retained.contains(&edge.target))
            .collect();
        matches.retain(|item| retained.contains(&item.node_id));
        response.concept_matches = matches;
        response
            .diagnostics
            .retain(|d| d.code != QueryDiagnosticCode::NoMatch);
        if let Some(first) = response.results.first() {
            response.diagnostics.push(QueryDiagnostic { code: QueryDiagnosticCode::NoMatch,
                message: "Approximate concept recall; synonym, document and semantic matches do not prove implementation equivalence. Exact structural evidence is unchanged.".to_owned(),
                node_id: Some(first.node_id.clone()), path: None });
        }
        self.finish_natural_response(response)
    }
}
