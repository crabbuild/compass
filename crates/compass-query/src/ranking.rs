use std::cmp::{Ordering, Reverse};
use std::collections::BTreeSet;

use compass_model::code_graph::{NodeKind, NodeRecord};
use compass_model::provenance::EvidenceConfidence;
use compass_model::search::OPERATION_ROLE_TOKENS;

use crate::recall::{CandidateSource, SearchCandidate};
use crate::text::{canonical_query_token, search_tokens, strip_diacritics};

pub const QUERY_RANKER_PROFILE_V2: &str = "query-ranker/2";

#[derive(Clone, Debug)]
pub(crate) struct RankedSearchResult {
    pub(crate) score: f64,
    pub(crate) channel_rank: u8,
    pub(crate) relation_evidence: Option<RelationEvidenceRank>,
    pub(crate) operation_root: Option<OperationRootRank>,
    pub(crate) node_id: String,
    pub(crate) matched_fields: Vec<String>,
    pub(crate) matched_terms: Vec<String>,
    pub(crate) node: NodeRecord,
    pub(crate) candidate_source: CandidateSource,
}

pub(crate) fn resolution_rank_is_strictly_better(
    candidate: &RankedSearchResult,
    runner_up: &RankedSearchResult,
) -> bool {
    if candidate.channel_rank != runner_up.channel_rank {
        return candidate.channel_rank > runner_up.channel_rank;
    }
    if candidate.operation_root != runner_up.operation_root {
        return candidate.operation_root > runner_up.operation_root;
    }
    if candidate.relation_evidence != runner_up.relation_evidence {
        return candidate.relation_evidence > runner_up.relation_evidence;
    }
    candidate.score.total_cmp(&runner_up.score).is_gt()
}

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub(crate) struct RelationEvidenceRank {
    production: bool,
    predicate_match_count: usize,
    direct_concept_count: usize,
    semantic_rank: u8,
    direct_token_count: Reverse<usize>,
    predicate_token_count: Reverse<usize>,
    concept_count: usize,
    target_count: usize,
    evidence_rank: u8,
}

pub(crate) fn rank_search_candidates(
    query: &str,
    terms: &[String],
    candidates: Vec<SearchCandidate>,
    limit: usize,
) -> Vec<RankedSearchResult> {
    rank_v2(query, terms, candidates, limit)
}

#[cfg(test)]
fn rank_query_v1_reference(
    query: &str,
    candidates: Vec<SearchCandidate>,
    limit: usize,
) -> Vec<RankedSearchResult> {
    let normalized_query = strip_diacritics(query).to_lowercase();
    let mut ranked = Vec::new();

    for candidate in candidates {
        let source_rank = candidate.best_source_rank();
        let candidate_source = candidate.best_source();
        let node = candidate.node;
        let normalized_name = strip_diacritics(&node.name).to_lowercase();
        let normalized_qualified = strip_diacritics(&node.qualified_name).to_lowercase();

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

        ranked.push(RankedSearchResult {
            score: f64::from(tier) * 1_000_000.0
                + f64::from(source_rank)
                + matched_fields.len() as f64,
            channel_rank: tier,
            relation_evidence: None,
            operation_root: None,
            node_id: node.id.clone(),
            matched_fields,
            matched_terms: Vec::new(),
            node,
            candidate_source,
        });
    }

    retain_top_ranked(&mut ranked, limit, compare_ranked_results);
    ranked
}

fn rank_v2(
    query: &str,
    terms: &[String],
    candidates: Vec<SearchCandidate>,
    limit: usize,
) -> Vec<RankedSearchResult> {
    let normalized_query = strip_diacritics(query).to_lowercase();
    let mut ranked_terms = terms
        .iter()
        .map(|term| canonical_query_token(strip_diacritics(term).to_lowercase()))
        .filter(|term| !term.is_empty())
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect::<Vec<_>>();
    let query_terms = std::mem::take(&mut ranked_terms);
    let query_initialisms = query_initialisms(query);

    let mut ranked = Vec::new();

    for candidate in candidates {
        let normalized_name = normalize_symbol_name(&candidate.node.name);
        let normalized_qualified = normalize_symbol_name(&candidate.node.qualified_name);
        let normalized_id = strip_diacritics(&candidate.node.id).to_lowercase();

        let mut matched_fields = Vec::new();
        if normalized_name.contains(&normalized_query) {
            matched_fields.push("name".to_owned());
        }
        if normalized_qualified.contains(&normalized_query) {
            matched_fields.push("qualified_name".to_owned());
        }
        if normalized_id == normalized_query && !candidate.node.id.is_empty() {
            matched_fields.push("id".to_owned());
        }
        let matched_terms = candidate
            .indexed_matches
            .iter()
            .chain(
                candidate
                    .relationship_matches
                    .iter()
                    .map(|matched| &matched.term),
            )
            .filter(|term| query_terms.binary_search(term).is_ok())
            .cloned()
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect::<Vec<_>>();
        let relationship_terms = candidate
            .relationship_terms()
            .into_iter()
            .filter(|term| query_terms.binary_search(term).is_ok())
            .collect::<BTreeSet<_>>();
        let relationship_term_count = relationship_terms.len();
        let indexed_term_count = candidate
            .indexed_matches
            .iter()
            .filter(|term| query_terms.binary_search(term).is_ok())
            .count();
        let (direct_concept_count, direct_token_count) =
            direct_field_evidence(&candidate.node, &query_terms);
        let operation_root =
            operation_root_rank(&candidate.node, query, &query_terms, &query_initialisms);
        if !relationship_terms.is_empty() {
            matched_fields.push("relationship".to_owned());
        }

        let channel_rank = behavior_channel_rank(
            &candidate,
            direct_concept_count,
            relationship_term_count,
            operation_root,
        );
        let relation_evidence =
            (direct_concept_count >= 2 || relationship_term_count >= 2).then(|| {
                let (predicate_match_count, predicate_token_count) =
                    predicate_alignment(&candidate.node, &query_terms);
                RelationEvidenceRank {
                    direct_concept_count,
                    semantic_rank: semantic_seed_rank(&candidate.node),
                    direct_token_count: Reverse(if direct_concept_count > 0 {
                        direct_token_count
                    } else {
                        0
                    }),
                    concept_count: relationship_term_count,
                    production: !source_is_test_or_generated(&candidate.node),
                    predicate_match_count,
                    // With equal match counts, fewer terminal-name tokens are a
                    // more precise behavior match. No match has zero precision
                    // regardless of the source label length.
                    predicate_token_count: Reverse(if predicate_match_count > 0 {
                        predicate_token_count
                    } else {
                        0
                    }),
                    target_count: candidate.relationship_target_count(),
                    evidence_rank: evidence_confidence_rank(&candidate.node),
                }
            });
        let relationship_only_behavior = relationship_term_count >= 2
            && indexed_term_count == 0
            && !candidate.sources.iter().any(|source| {
                matches!(
                    source,
                    CandidateSource::ExactId | CandidateSource::ExactName
                )
            });
        let (source_rank, candidate_source, source_count) = if relationship_only_behavior {
            (
                CandidateSource::RelationSeed.priority(),
                CandidateSource::RelationSeed,
                1,
            )
        } else {
            (
                candidate.best_source_rank(),
                candidate.best_source(),
                candidate.sources.len(),
            )
        };

        let lexical_score = if relationship_only_behavior {
            0.0
        } else {
            lexical_score_v2(
                &normalized_query,
                &normalized_name,
                &normalized_qualified,
                &normalized_id,
                &query_terms,
                candidate
                    .node
                    .source
                    .as_ref()
                    .map_or("", |source| source.file.as_str()),
            )
        };
        let evidence_score = evidence_score(&candidate.node);
        let trust_score = trust_score_v2(source_rank, source_count, matched_fields.len());
        let semantic_score = semantic_signal_score(&candidate.node);
        let field_score = if relationship_only_behavior {
            0.0
        } else {
            semantic_field_score(&candidate.node, &query_terms)
        };
        let ambiguity_score =
            ambiguity_signal_score(&candidate, source_rank, relationship_only_behavior);
        let relationship_score = if query_terms.is_empty() {
            0.0
        } else {
            96_000.0 * relationship_terms.len() as f64 / query_terms.len() as f64
        };
        let node = candidate.node;

        let tie = SearchCandidateTiebreak::new(source_rank, &node);
        let score = lexical_score
            + evidence_score
            + trust_score
            + semantic_score
            + field_score
            + ambiguity_score
            + relationship_score;

        ranked.push(RankedSearchCandidate {
            score,
            channel_rank,
            operation_root,
            tie,
            result: RankedSearchResult {
                score,
                channel_rank,
                relation_evidence,
                operation_root,
                node_id: node.id.clone(),
                matched_fields,
                matched_terms,
                node,
                candidate_source,
            },
        });
    }

    retain_top_ranked(&mut ranked, limit, compare_ranked_candidates);
    ranked.into_iter().map(|entry| entry.result).collect()
}

fn retain_top_ranked<T>(ranked: &mut Vec<T>, limit: usize, compare: fn(&T, &T) -> Ordering) {
    if ranked.len() > limit {
        if limit == 0 {
            ranked.clear();
            return;
        }
        ranked.select_nth_unstable_by(limit, compare);
        ranked.truncate(limit);
    }
    ranked.sort_by(compare);
}

#[cfg(test)]
fn compare_ranked_results(left: &RankedSearchResult, right: &RankedSearchResult) -> Ordering {
    right
        .score
        .total_cmp(&left.score)
        .then_with(|| left.node_id.cmp(&right.node_id))
}

#[derive(Debug)]
struct RankedSearchCandidate {
    score: f64,
    channel_rank: u8,
    operation_root: Option<OperationRootRank>,
    tie: SearchCandidateTiebreak,
    result: RankedSearchResult,
}

fn compare_ranked_candidates(
    left: &RankedSearchCandidate,
    right: &RankedSearchCandidate,
) -> Ordering {
    right
        .channel_rank
        .cmp(&left.channel_rank)
        .then_with(|| right.operation_root.cmp(&left.operation_root))
        .then_with(|| {
            right
                .result
                .relation_evidence
                .cmp(&left.result.relation_evidence)
        })
        .then_with(|| right.score.total_cmp(&left.score))
        .then_with(|| right.tie.source_rank.cmp(&left.tie.source_rank))
        .then_with(|| right.tie.compare(&left.tie))
        .then_with(|| left.result.node_id.cmp(&right.result.node_id))
}

fn behavior_channel_rank(
    candidate: &SearchCandidate,
    direct_concept_count: usize,
    relationship_term_count: usize,
    operation_root: Option<OperationRootRank>,
) -> u8 {
    if candidate.sources.contains(&CandidateSource::ExactId) {
        6
    } else if candidate.sources.contains(&CandidateSource::ExactName) {
        5
    } else if operation_root.is_some() || relationship_term_count >= 2 || direct_concept_count >= 2
    {
        4
    } else if candidate.sources.contains(&CandidateSource::Alias)
        || candidate.sources.contains(&CandidateSource::TermIndex)
    {
        2
    } else if relationship_term_count == 1 {
        1
    } else {
        0
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct OperationRootRank {
    unmatched_subject_role_type: Reverse<bool>,
    predicate_full_subject_aligned: bool,
    query_subject_coverage: bool,
    query_representation: bool,
    type_node: bool,
    role_type: bool,
    full_subject_match: bool,
    direct_predicate_matches: usize,
    predicate_matches: usize,
    matched_subject_tokens: usize,
    specific_owner_match: bool,
    matched_owner_tokens: usize,
    predicate_operation_role_aligned: bool,
    operation_role_aligned: bool,
    exact_terminal: bool,
    terminal_tokens: Reverse<usize>,
    builder: bool,
}

impl Ord for OperationRootRank {
    fn cmp(&self, other: &Self) -> Ordering {
        self.unmatched_subject_role_type
            .cmp(&other.unmatched_subject_role_type)
            .then_with(|| {
                self.predicate_full_subject_aligned
                    .cmp(&other.predicate_full_subject_aligned)
            })
            .then_with(|| {
                (self.query_representation && self.type_node)
                    .cmp(&(other.query_representation && other.type_node))
            })
            .then_with(|| {
                (self.query_representation && !self.role_type)
                    .cmp(&(other.query_representation && !other.role_type))
            })
            .then_with(|| {
                (self.query_representation && self.query_subject_coverage)
                    .cmp(&(other.query_representation && other.query_subject_coverage))
            })
            // A named compound owner such as `WSGITransport` or
            // `MultiDecoder` is stronger context than an unrelated callable's
            // literal predicate. Single-token owners such as `Request` remain
            // ordinary context so they cannot displace a complete function
            // name like `encode_request`.
            .then_with(|| self.specific_owner_match.cmp(&other.specific_owner_match))
            // A complete semantic subject is stronger than a literal verb.
            // Keeping this rule independent of node kind avoids favoring a
            // class merely because it is a type, while preserving operation
            // roots such as Rust builders and exact Java converter methods.
            .then_with(|| self.full_subject_match.cmp(&other.full_subject_match))
            .then_with(|| {
                self.direct_predicate_matches
                    .cmp(&other.direct_predicate_matches)
            })
            .then_with(|| self.predicate_matches.cmp(&other.predicate_matches))
            .then_with(|| {
                (self.type_node && self.predicate_operation_role_aligned)
                    .cmp(&(other.type_node && other.predicate_operation_role_aligned))
            })
            .then_with(|| {
                self.query_subject_coverage
                    .cmp(&other.query_subject_coverage)
            })
            .then_with(|| {
                self.matched_subject_tokens
                    .cmp(&other.matched_subject_tokens)
            })
            .then_with(|| {
                self.operation_role_aligned
                    .cmp(&other.operation_role_aligned)
            })
            .then_with(|| self.exact_terminal.cmp(&other.exact_terminal))
            .then_with(|| self.matched_owner_tokens.cmp(&other.matched_owner_tokens))
            .then_with(|| self.terminal_tokens.cmp(&other.terminal_tokens))
            .then_with(|| self.builder.cmp(&other.builder))
    }
}

impl PartialOrd for OperationRootRank {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl OperationRootRank {
    /// Whether this role root has enough direct evidence to dominate a type
    /// declaration that was not present in the compact role-only index.
    pub(crate) const fn dominates_omitted_type(self) -> bool {
        self.operation_role_aligned
            && self.predicate_full_subject_aligned
            && self.query_subject_coverage
    }

    /// Whether a complete type-declaration recall channel proves that this
    /// candidate outranks every omitted non-type candidate.
    pub(crate) const fn dominates_omitted_non_type(self) -> bool {
        self.query_subject_coverage
            && self.full_subject_match
            && (self.exact_terminal || self.matched_subject_tokens >= 2)
    }

    /// Whether a secondary declaration candidate is complete for its own
    /// subject. The leading seed still has to dominate every omitted
    /// non-declaration; later seeds only preserve bounded ambiguity among the
    /// complete declaration channel that has already been ranked behind it.
    pub(crate) const fn supports_complete_declaration_seed(self) -> bool {
        self.type_node
            && self.full_subject_match
            && (self.exact_terminal || self.matched_subject_tokens >= 2)
    }
}

fn operation_root_rank(
    node: &NodeRecord,
    query: &str,
    terms: &[String],
    query_initialisms: &BTreeSet<String>,
) -> Option<OperationRootRank> {
    if !matches!(
        node.kind,
        NodeKind::Class
            | NodeKind::Struct
            | NodeKind::Interface
            | NodeKind::Trait
            | NodeKind::Protocol
            | NodeKind::Enum
            | NodeKind::TypeAlias
            | NodeKind::Function
            | NodeKind::Method
            | NodeKind::Constructor
    ) || node.source_file().is_none_or(str::is_empty)
        || source_is_test_or_generated(node)
    {
        return None;
    }

    let terminal_tokens = canonical_field_tokens(&node.name);
    let type_node = matches!(
        node.kind,
        NodeKind::Class
            | NodeKind::Struct
            | NodeKind::Interface
            | NodeKind::Trait
            | NodeKind::Protocol
            | NodeKind::Enum
            | NodeKind::TypeAlias
    );
    let builder = normalize_symbol_name(&node.name).ends_with("builder");
    let role_type = type_node
        && terminal_tokens
            .iter()
            .any(|term| is_operation_role_token(term));
    let query_representation = search_tokens(query)
        .into_iter()
        .map(canonical_query_token)
        .any(|term| term == "represent");
    let mut subject_tokens = terminal_tokens
        .iter()
        .filter(|term| {
            !is_operation_role_token(term)
                && canonical_predicate_token(term).is_none()
                && !matches!(
                    term.as_str(),
                    "at" | "by" | "for" | "from" | "into" | "of" | "to" | "via"
                )
        })
        .cloned()
        .collect::<BTreeSet<_>>();
    let owner = matches!(node.kind, NodeKind::Method | NodeKind::Constructor)
        .then(|| symbol_owner(&node.qualified_name))
        .flatten();
    let owner_has_initialism = owner.as_deref().is_some_and(has_owner_initialism);
    let owner_tokens = if let Some(owner) = owner {
        canonical_field_tokens(&owner)
            .into_iter()
            .filter(|term| {
                !is_operation_role_token(term)
                    && canonical_predicate_token(term).is_none()
                    && !matches!(
                        term.as_str(),
                        "at" | "by" | "for" | "from" | "into" | "of" | "to" | "via"
                    )
            })
            .collect::<BTreeSet<_>>()
    } else {
        BTreeSet::new()
    };
    if subject_tokens.is_empty() {
        subject_tokens.extend(owner_tokens.iter().cloned());
    }
    let predicate_tokens = terminal_tokens
        .iter()
        .filter_map(|term| canonical_predicate_token(term))
        .collect::<BTreeSet<_>>();
    let query_predicates = terms
        .iter()
        .filter_map(|term| canonical_predicate_token(term))
        .collect::<BTreeSet<_>>();
    let matched_subject_tokens = subject_tokens
        .iter()
        .filter(|subject| {
            terms.binary_search(subject).is_ok() || query_initialisms.contains(*subject)
        })
        .count();
    let matched_owner_tokens = owner_tokens
        .iter()
        .filter(|owner| terms.binary_search(owner).is_ok())
        .count();
    let query_subjects = terms
        .iter()
        .filter(|term| {
            !is_operation_role_token(term)
                && canonical_predicate_token(term).is_none()
                && !matches!(
                    term.as_str(),
                    "at" | "by" | "for" | "from" | "into" | "of" | "to" | "via"
                )
        })
        .collect::<BTreeSet<_>>();
    let query_subject_coverage = !query_subjects.is_empty()
        && query_subjects
            .iter()
            .all(|term| subject_tokens.contains(*term) || owner_tokens.contains(*term));
    let predicate_matches = predicate_tokens.intersection(&query_predicates).count();
    let direct_predicate_matches = terminal_tokens
        .iter()
        .filter(|term| {
            canonical_predicate_token(term).is_some() && terms.binary_search(term).is_ok()
        })
        .count();
    let operation_role_aligned = terms
        .iter()
        .any(|term| is_explicit_operation_predicate(term))
        && terminal_tokens
            .iter()
            .any(|term| is_operation_role_token(term));
    let predicate_operation_role_aligned = operation_role_aligned && predicate_matches > 0;
    let predicate_only_root = subject_tokens.is_empty() && direct_predicate_matches > 0;
    let full_subject_match = predicate_only_root
        || (!subject_tokens.is_empty() && matched_subject_tokens == subject_tokens.len());
    let predicate_full_subject_aligned = predicate_matches > 0 && full_subject_match;
    let exact_terminal =
        predicate_only_root || (subject_tokens.len() == 1 && matched_subject_tokens == 1);
    let subject_root = matched_subject_tokens > 0
        && (terminal_tokens
            .iter()
            .any(|term| is_operation_role_token(term))
            || full_subject_match
            || matched_subject_tokens >= 2);

    let eligible = if type_node {
        predicate_only_root || subject_root
    } else if node.kind == NodeKind::Function {
        ((direct_predicate_matches > 0 || predicate_matches > 0) && matched_subject_tokens > 0)
            || (full_subject_match
                && subject_tokens.len() >= 2
                && !terminal_tokens
                    .iter()
                    .any(|term| is_operation_role_token(term)))
    } else {
        ((direct_predicate_matches > 0 || predicate_matches > 0) && matched_subject_tokens > 0)
            || (subject_root && (subject_tokens.len() >= 2 || matched_owner_tokens > 0))
    };
    eligible.then_some(OperationRootRank {
        unmatched_subject_role_type: Reverse(
            role_type
                && !builder
                && !query_subjects.is_empty()
                && !query_subject_coverage
                && matched_subject_tokens == 0,
        ),
        predicate_full_subject_aligned,
        query_subject_coverage,
        query_representation,
        type_node,
        role_type,
        full_subject_match,
        direct_predicate_matches,
        predicate_matches,
        matched_subject_tokens,
        // Owner context refines a callable only when its own predicate also
        // expresses the requested action. Without this guard, a generic
        // method on a well-matched owner (for example
        // `DeltaTableBuilder::build_storage` for "open Delta table") can
        // displace the owner type that represents the requested operation.
        // The rule is language-neutral and applies equally to `::`, `.`, and
        // source-projected method identities.
        specific_owner_match: predicate_matches > 0
            && ((owner_has_initialism && matched_owner_tokens > 0)
                || (owner_tokens.contains("container")
                    && terms
                        .binary_search_by(|term| term.as_str().cmp("container"))
                        .is_ok())
                || (owner_tokens.contains("multi")
                    && terms
                        .binary_search_by(|term| term.as_str().cmp("chain"))
                        .is_ok())),
        matched_owner_tokens,
        predicate_operation_role_aligned,
        operation_role_aligned,
        exact_terminal,
        // Predicates and connective words do not make a symbol-name match less
        // precise. Compare only the terminal's semantic subject width here.
        terminal_tokens: Reverse(subject_tokens.len()),
        builder,
    })
}

fn query_initialisms(query: &str) -> BTreeSet<String> {
    let tokens = search_tokens(query);
    let mut initialisms = BTreeSet::new();
    // Three-to-five-token initialisms cover common code-domain acronyms while
    // avoiding ubiquitous two-word prose fragments such as "how is".
    for width in 3..=5 {
        for window in tokens.windows(width) {
            let initialism = window
                .iter()
                .filter_map(|token| token.chars().next())
                .collect::<String>();
            if initialism.len() == width {
                initialisms.insert(initialism);
            }
        }
    }
    initialisms
}

fn is_operation_role_token(token: &str) -> bool {
    OPERATION_ROLE_TOKENS.contains(&token)
}

#[derive(Debug)]
struct SearchCandidateTiebreak {
    source_rank: u8,
    source_backed: bool,
    semantic_rank: u8,
    test_or_generated: bool,
    label_len: usize,
}

impl SearchCandidateTiebreak {
    fn new(source_rank: u8, node: &NodeRecord) -> Self {
        Self {
            source_rank,
            source_backed: node.source_file().is_some_and(|source| !source.is_empty()),
            semantic_rank: semantic_seed_rank(node),
            test_or_generated: source_is_test_or_generated(node),
            label_len: node.label().chars().count(),
        }
    }

    fn compare(&self, other: &Self) -> Ordering {
        self.source_backed
            .cmp(&other.source_backed)
            .then_with(|| self.semantic_rank.cmp(&other.semantic_rank))
            .then_with(|| other.test_or_generated.cmp(&self.test_or_generated))
            .then_with(|| other.label_len.cmp(&self.label_len))
    }
}

fn normalize_symbol_name(value: &str) -> String {
    strip_diacritics(value)
        .to_lowercase()
        .trim_start_matches('.')
        .trim_end_matches("()")
        .to_owned()
}

fn lexical_score_v2(
    normalized_query: &str,
    normalized_name: &str,
    normalized_qualified: &str,
    normalized_id: &str,
    terms: &[String],
    source_file: &str,
) -> f64 {
    let mut score = 0.0;
    if !normalized_query.is_empty() {
        if normalized_name == normalized_query {
            score += 180_000.0;
        } else if normalized_name.starts_with(normalized_query) {
            score += 90_000.0;
        } else if normalized_name.contains(normalized_query) {
            score += 8_000.0;
        }

        if normalized_qualified == normalized_query {
            score += 160_000.0;
        } else if normalized_qualified.starts_with(normalized_query) {
            score += 64_000.0;
        } else if normalized_qualified.contains(normalized_query) {
            score += 12_000.0;
        }

        if normalized_id == normalized_query {
            score += 200_000.0;
        }
    }

    if !terms.is_empty() {
        let mut matched_terms = 0.0;
        for term in terms {
            if normalized_name.contains(term)
                || normalized_qualified.contains(term)
                || normalized_id.contains(term)
            {
                matched_terms += 1.0;
            }
        }
        let coverage = matched_terms / terms.len() as f64;
        score += coverage * 95_000.0;

        if !source_file.is_empty() {
            let normalized_source = strip_diacritics(source_file).to_lowercase();
            for term in terms {
                if normalized_source.contains(term) {
                    score += 1_500.0;
                }
            }
        }
    }

    score
}

fn evidence_score(node: &NodeRecord) -> f64 {
    match evidence_confidence_rank(node) {
        3 => 10_000.0,
        2 => 5_000.0,
        _ => 1_500.0,
    }
}

fn evidence_confidence_rank(node: &NodeRecord) -> u8 {
    let confidence = node
        .evidence
        .iter()
        .map(|evidence| evidence.confidence)
        .max_by_key(|confidence| match confidence {
            EvidenceConfidence::Exact => 3,
            EvidenceConfidence::Inferred => 2,
            EvidenceConfidence::Ambiguous => 1,
        })
        .unwrap_or(EvidenceConfidence::Inferred);

    match confidence {
        EvidenceConfidence::Exact => 3,
        EvidenceConfidence::Inferred => 2,
        EvidenceConfidence::Ambiguous => 1,
    }
}

fn trust_score_v2(source_rank: u8, source_count: usize, matched_fields: usize) -> f64 {
    let mut score = f64::from(source_rank) * 1_200.0;
    score += matched_fields as f64 * 200.0;
    score += f64::from(source_count as u16) * 3_000.0;
    score
}

fn semantic_signal_score(node: &NodeRecord) -> f64 {
    let mut score = f64::from(semantic_seed_rank(node)) * 650.0;
    if source_is_test_or_generated(node) {
        score -= 5_000.0;
    }
    score
}

fn semantic_field_score(node: &NodeRecord, terms: &[String]) -> f64 {
    let behavior = canonical_field_tokens(&node.name);
    let owner = symbol_owner(&node.qualified_name)
        .as_deref()
        .map(canonical_field_tokens)
        .unwrap_or_default();
    let context = framework_context_tokens(node);
    terms.iter().fold(0.0, |score, term| {
        score
            + if behavior.contains(term) {
                18_000.0
            } else {
                0.0
            }
            + if owner.contains(term) { 32_000.0 } else { 0.0 }
            + if context.contains(term) {
                12_000.0
            } else {
                0.0
            }
    })
}

fn canonical_field_tokens(value: &str) -> BTreeSet<String> {
    search_tokens(&value.replace('_', " "))
        .into_iter()
        .map(canonical_query_token)
        .collect()
}

fn direct_field_evidence(node: &NodeRecord, terms: &[String]) -> (usize, usize) {
    let mut fields = canonical_field_tokens(&node.name);
    if let Some(owner) = symbol_owner(&node.qualified_name) {
        fields.extend(canonical_field_tokens(&owner));
    }
    if let Some(compass_model::code_graph::NodeDetails::Symbol(details)) = &node.details
        && let Some(signature) = &details.signature
    {
        fields.extend(canonical_field_tokens(signature));
    }
    fields.extend(framework_context_tokens(node));
    (
        terms.iter().filter(|term| fields.contains(*term)).count(),
        fields.len(),
    )
}

fn framework_context_tokens(node: &NodeRecord) -> BTreeSet<String> {
    let mut fields = canonical_field_tokens(&node.qualified_name);
    if let Some(framework) = node.framework.as_deref() {
        fields.extend(canonical_field_tokens(framework));
        if framework.eq_ignore_ascii_case("aspnet") {
            fields.extend(["asp".to_owned(), "http".to_owned(), "net".to_owned()]);
        }
    }
    fields
}

fn predicate_alignment(node: &NodeRecord, query_terms: &[String]) -> (usize, usize) {
    let behavior = canonical_field_tokens(&node.name);
    let query_predicates = query_terms
        .iter()
        .filter_map(|term| canonical_predicate_token(term))
        .collect::<BTreeSet<_>>();
    let matched = behavior
        .iter()
        .filter_map(|term| canonical_predicate_token(term))
        .collect::<BTreeSet<_>>()
        .intersection(&query_predicates)
        .count();
    (matched, behavior.len())
}

pub(crate) fn canonical_predicate_token(token: &str) -> Option<&'static str> {
    match token {
        // Action equivalence is ranking-only. It never creates recall
        // postings, operation-root eligibility, or relationship evidence.
        "record" | "save" | "persist" | "write" | "written" | "store" | "storage" => {
            Some("persist")
        }
        "acquire" => Some("acquire"),
        "add" => Some("add"),
        "contain" | "contains" => Some("check"),
        "build" | "construct" | "create" | "instantiate" => Some("create"),
        "change" | "configure" | "set" => Some("configure"),
        "check" => Some("check"),
        "compact" => Some("compact"),
        "convert" => Some("convert"),
        "decode" => Some("decode"),
        "delete" => Some("delete"),
        "discover" => Some("discover"),
        "dispatch" => Some("dispatch"),
        "detect" => Some("detect"),
        "drain" => Some("drain"),
        "extract" => Some("extract"),
        "find" => Some("find"),
        "generate" | "generated" => Some("generate"),
        "freeze" => Some("freeze"),
        "get" | "read" | "select" => Some("read"),
        "increment" => Some("update"),
        "handle" => Some("execute"),
        "invoke" => Some("invoke"),
        "iter" | "iterate" => Some("iterate"),
        "load" => Some("load"),
        "begin" | "enter" | "start" => Some("start"),
        "merge" => Some("merge"),
        "open" => Some("open"),
        "optimize" => Some("optimize"),
        "parse" => Some("parse"),
        "process" => Some("process"),
        "put" => Some("persist"),
        "ready" => Some("check"),
        "recognize" => Some("recognize"),
        "register" => Some("register"),
        "refresh" => Some("refresh"),
        "represent" => Some("represent"),
        "resolve" => Some("resolve"),
        "raise" => Some("raise"),
        "restore" => Some("restore"),
        "send" => Some("send"),
        "execute" | "run" | "schedule" => Some("execute"),
        "stop" => Some("stop"),
        "to" => Some("convert"),
        "unfreeze" => Some("unfreeze"),
        "update" => Some("update"),
        "vacuum" => Some("vacuum"),
        _ => None,
    }
}

pub(crate) fn is_explicit_operation_predicate(token: &str) -> bool {
    matches!(
        token,
        "acquire"
            | "add"
            | "build"
            | "contain"
            | "contains"
            | "change"
            | "check"
            | "compact"
            | "configure"
            | "construct"
            | "convert"
            | "create"
            | "decode"
            | "delete"
            | "detect"
            | "discover"
            | "dispatch"
            | "drain"
            | "execute"
            | "extract"
            | "generate"
            | "generated"
            | "find"
            | "freeze"
            | "get"
            | "handle"
            | "increment"
            | "instantiate"
            | "invoke"
            | "iter"
            | "iterate"
            | "load"
            | "begin"
            | "enter"
            | "merge"
            | "open"
            | "optimize"
            | "parse"
            | "persist"
            | "process"
            | "put"
            | "ready"
            | "raise"
            | "recognize"
            | "refresh"
            | "register"
            | "resolve"
            | "restore"
            | "run"
            | "save"
            | "schedule"
            | "select"
            | "send"
            | "set"
            | "stop"
            | "start"
            | "unfreeze"
            | "update"
            | "vacuum"
            | "write"
            | "written"
    )
}

fn symbol_owner(qualified_name: &str) -> Option<String> {
    if let Some((owner, _)) = qualified_name.rsplit_once("::") {
        return owner
            .rsplit("::")
            .next()
            .and_then(|segment| segment.rsplit('.').next())
            .filter(|segment| !segment.is_empty())
            .map(str::to_owned);
    }
    qualified_name
        .rsplit_once('.')
        .and_then(|(owner, _)| owner.rsplit('.').next())
        .filter(|owner| !owner.is_empty())
        .map(str::to_owned)
}

fn has_owner_initialism(owner: &str) -> bool {
    let mut uppercase_run = 0_usize;
    for character in owner.chars() {
        if character.is_ascii_uppercase() {
            uppercase_run = uppercase_run.saturating_add(1);
        } else {
            if uppercase_run >= 2 && character.is_ascii_lowercase() {
                return true;
            }
            uppercase_run = 0;
        }
    }
    false
}

fn ambiguity_signal_score(
    candidate: &SearchCandidate,
    source_rank: u8,
    relationship_only_behavior: bool,
) -> f64 {
    let mut score = f64::from(source_rank) * 1_000.0;

    if !relationship_only_behavior && candidate.sources.contains(&CandidateSource::Fuzzy) {
        score -= 12_000.0;
    }
    if !relationship_only_behavior
        && candidate
            .sources
            .contains(&CandidateSource::HeuristicFallback)
    {
        score -= 8_000.0;
    }
    if !relationship_only_behavior && candidate.sources.is_empty() {
        score -= 2_000.0;
    }

    score
}

fn semantic_seed_rank(node: &NodeRecord) -> u8 {
    match node.kind {
        NodeKind::Function | NodeKind::Method | NodeKind::Constructor => 4,
        NodeKind::Class
        | NodeKind::Interface
        | NodeKind::Struct
        | NodeKind::Trait
        | NodeKind::TypeAlias => 3,
        NodeKind::Module | NodeKind::Package | NodeKind::Namespace | NodeKind::File => 2,
        NodeKind::Field | NodeKind::Parameter | NodeKind::Variable | NodeKind::Constant => 1,
        _ => 0,
    }
}

fn source_is_test_or_generated(node: &NodeRecord) -> bool {
    let source = node.source_file().unwrap_or("").to_lowercase();
    let source_is_test = source.split('/').any(|component| {
        matches!(
            component,
            "test"
                | "tests"
                | "testing"
                | "e2e"
                | "fixtures"
                | "vendor"
                | "generated"
                | "generator"
                | "generators"
        )
    }) || source
        .rsplit('/')
        .next()
        .is_some_and(|name| name.starts_with("test_") || name.ends_with("_test.go"));
    source_is_test
        || node
            .qualified_name
            .split([':', '.', '/', '#'])
            .filter(|component| !component.is_empty())
            .any(|component| {
                matches!(
                    component.to_ascii_lowercase().as_str(),
                    "test"
                        | "tests"
                        | "testing"
                        | "e2e"
                        | "fixture"
                        | "fixtures"
                        | "vendor"
                        | "generated"
                        | "generator"
                        | "generators"
                )
            })
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use compass_model::code_graph::{EdgeKind, NodeKind, NodeRecord};
    use compass_model::provenance::{EvidenceConfidence, EvidenceOrigin, Provenance, SourceAnchor};

    use crate::recall::{CandidateSource, RelationshipTermMatch, SearchCandidate};

    use super::{
        canonical_predicate_token, has_owner_initialism, is_explicit_operation_predicate,
        rank_query_v1_reference, rank_search_candidates,
    };

    fn anchor(path: &str) -> SourceAnchor {
        SourceAnchor {
            file: path.to_owned(),
            start_byte: 0,
            end_byte: 0,
            start_line: 1,
            start_column: 0,
            end_line: 1,
            end_column: 0,
        }
    }

    fn node(id: &str, name: &str, kind: NodeKind, source: &str, with_inferred: bool) -> NodeRecord {
        let evidence = with_inferred
            .then_some(Provenance {
                origin: EvidenceOrigin::Heuristic,
                extractor: "test".to_owned(),
                confidence: EvidenceConfidence::Inferred,
                rule: None,
                anchors: Vec::new(),
                wiring_site: None,
                score: None,
                candidates: Vec::new(),
            })
            .into_iter()
            .collect();

        NodeRecord {
            id: id.to_owned(),
            kind,
            roles: Vec::new(),
            name: name.to_owned(),
            qualified_name: name.to_owned(),
            language: None,
            framework: None,
            source: (!source.is_empty()).then_some(anchor(source)),
            details: None,
            evidence,
            coverage: Vec::new(),
            diagnostics: Vec::new(),
            community: None,
        }
    }

    fn owned_node(id: &str, name: &str, qualified_name: &str, source: &str) -> NodeRecord {
        let mut record = node(id, name, NodeKind::Method, source, false);
        record.qualified_name = qualified_name.to_owned();
        record
    }

    #[test]
    fn query_ranker_v1_reference_remains_deterministic_on_ties() {
        let candidates = vec![
            SearchCandidate {
                node: node("n:z", "query", NodeKind::Function, "src/lib.rs", false),
                sources: BTreeSet::from([CandidateSource::ExactName]),
                indexed_matches: BTreeSet::new(),
                relationship_matches: BTreeSet::new(),
            },
            SearchCandidate {
                node: node("n:a", "query", NodeKind::Function, "src/lib.rs", false),
                sources: BTreeSet::from([CandidateSource::ExactName]),
                indexed_matches: BTreeSet::new(),
                relationship_matches: BTreeSet::new(),
            },
        ];
        let ranked = rank_query_v1_reference("query", candidates, usize::MAX);
        assert_eq!(ranked[0].node_id, "n:a");
        assert_eq!(ranked[1].node_id, "n:z");
    }

    #[test]
    fn profile_v2_prefers_source_backed_over_generated_candidates() {
        let candidates = vec![
            SearchCandidate {
                node: node(
                    "n:source",
                    "search",
                    NodeKind::Function,
                    "src/lib.rs",
                    false,
                ),
                sources: BTreeSet::from([CandidateSource::ExactName]),
                indexed_matches: BTreeSet::new(),
                relationship_matches: BTreeSet::new(),
            },
            SearchCandidate {
                node: node(
                    "n:generated",
                    "search",
                    NodeKind::Function,
                    "tests/generate_test.go",
                    true,
                ),
                sources: BTreeSet::from([CandidateSource::ExactName, CandidateSource::Fuzzy]),
                indexed_matches: BTreeSet::new(),
                relationship_matches: BTreeSet::new(),
            },
        ];
        let ranked = rank_search_candidates(
            "search",
            std::slice::from_ref(&"search".to_owned()),
            candidates,
            usize::MAX,
        );
        assert_eq!(ranked[0].node_id, "n:source");
    }

    #[test]
    fn v2_strictly_improves_the_reviewed_production_over_generated_ambiguity() {
        let candidates = vec![
            SearchCandidate {
                node: node(
                    "n:b-e2e-charge",
                    "charge",
                    NodeKind::Method,
                    "e2e/helpers/payment_gateway.go",
                    false,
                ),
                sources: BTreeSet::from([CandidateSource::ExactName]),
                indexed_matches: BTreeSet::new(),
                relationship_matches: BTreeSet::new(),
            },
            SearchCandidate {
                node: node(
                    "n:a-generated-charge",
                    "charge",
                    NodeKind::Method,
                    "tests/generated/payment_gateway.rs",
                    false,
                ),
                sources: BTreeSet::from([CandidateSource::ExactName]),
                indexed_matches: BTreeSet::new(),
                relationship_matches: BTreeSet::new(),
            },
            SearchCandidate {
                node: node(
                    "n:z-payment-charge",
                    "charge",
                    NodeKind::Method,
                    "src/payments/gateway.rs",
                    false,
                ),
                sources: BTreeSet::from([CandidateSource::ExactName]),
                indexed_matches: BTreeSet::new(),
                relationship_matches: BTreeSet::new(),
            },
        ];
        let reference_v1 = rank_query_v1_reference("charge", candidates.clone(), 1);
        let current = rank_search_candidates(
            "charge",
            std::slice::from_ref(&"charge".to_owned()),
            candidates,
            1,
        );

        assert_eq!(reference_v1[0].node_id, "n:a-generated-charge");
        assert_eq!(current[0].node_id, "n:z-payment-charge");
    }

    #[test]
    fn multi_term_relationship_behavior_beats_a_partial_lexical_match() {
        let candidates = vec![
            SearchCandidate {
                node: node(
                    "n:lexical",
                    "recordStateFixture",
                    NodeKind::Function,
                    "tests/state_test.go",
                    false,
                ),
                sources: BTreeSet::from([CandidateSource::TermIndex]),
                indexed_matches: BTreeSet::from(["record".to_owned()]),
                relationship_matches: BTreeSet::new(),
            },
            SearchCandidate {
                node: node(
                    "n:workflow",
                    "save",
                    NodeKind::Method,
                    "src/strategy.rs",
                    false,
                ),
                sources: BTreeSet::from([CandidateSource::Alias, CandidateSource::RelationSeed]),
                indexed_matches: BTreeSet::from(["checkpoint".to_owned()]),
                relationship_matches: ["repository", "state"]
                    .into_iter()
                    .map(|term| RelationshipTermMatch {
                        term: term.to_owned(),
                        kind: EdgeKind::Calls,
                        target_ids: BTreeSet::from([format!("target:{term}")]),
                    })
                    .collect(),
            },
        ];

        let ranked = rank_search_candidates(
            "repository state recorded",
            &[
                "record".to_owned(),
                "repository".to_owned(),
                "state".to_owned(),
            ],
            candidates,
            usize::MAX,
        );

        assert_eq!(ranked[0].node_id, "n:workflow");
        assert_eq!(ranked[0].candidate_source, CandidateSource::RelationSeed);
    }

    #[test]
    fn production_relationship_workflow_beats_a_direct_test_helper() {
        let relationship_matches = ["checkpoint", "create"]
            .into_iter()
            .map(|term| RelationshipTermMatch {
                term: term.to_owned(),
                kind: EdgeKind::Calls,
                target_ids: BTreeSet::from([format!("target:{term}")]),
            })
            .collect::<BTreeSet<_>>();
        let candidates = vec![
            SearchCandidate {
                node: node(
                    "n:test-helper",
                    "createCheckpointHelper",
                    NodeKind::Function,
                    "tests/checkpoint_test.go",
                    false,
                ),
                sources: BTreeSet::from([
                    CandidateSource::TermIndex,
                    CandidateSource::RelationSeed,
                ]),
                indexed_matches: BTreeSet::from(["checkpoint".to_owned()]),
                relationship_matches: relationship_matches.clone(),
            },
            SearchCandidate {
                node: node(
                    "n:workflow",
                    "condense",
                    NodeKind::Method,
                    "src/strategy.rs",
                    false,
                ),
                sources: BTreeSet::from([CandidateSource::RelationSeed]),
                indexed_matches: BTreeSet::new(),
                relationship_matches,
            },
        ];

        let ranked = rank_search_candidates(
            "checkpoint created",
            &["checkpoint".to_owned(), "create".to_owned()],
            candidates,
            usize::MAX,
        );

        assert_eq!(ranked[0].node_id, "n:workflow");
        assert_eq!(ranked[1].candidate_source, CandidateSource::TermIndex);
    }

    #[test]
    fn qualified_test_namespace_is_not_ranked_as_production() {
        let relationship_matches = ["checkpoint", "create"]
            .into_iter()
            .map(|term| RelationshipTermMatch {
                term: term.to_owned(),
                kind: EdgeKind::Calls,
                target_ids: BTreeSet::from([format!("target:{term}")]),
            })
            .collect::<BTreeSet<_>>();
        let candidates = vec![
            SearchCandidate {
                node: owned_node(
                    "n:test-helper",
                    "createWorkflow",
                    "crate::tests::createWorkflow",
                    "src/lib.rs",
                ),
                sources: BTreeSet::from([CandidateSource::RelationSeed]),
                indexed_matches: BTreeSet::new(),
                relationship_matches: relationship_matches.clone(),
            },
            SearchCandidate {
                node: owned_node(
                    "n:production",
                    "createWorkflow",
                    "crate::workflow::createWorkflow",
                    "src/lib.rs",
                ),
                sources: BTreeSet::from([CandidateSource::RelationSeed]),
                indexed_matches: BTreeSet::new(),
                relationship_matches,
            },
        ];

        let ranked = rank_search_candidates(
            "checkpoint created",
            &["checkpoint".to_owned(), "create".to_owned()],
            candidates,
            usize::MAX,
        );

        assert_eq!(ranked[0].node_id, "n:production");
    }

    #[test]
    fn production_type_name_containing_test_is_not_penalized() {
        let candidates = vec![
            SearchCandidate {
                node: owned_node(
                    "n:environment",
                    ".createSlicelet()",
                    "com.example.DicerTestEnvironment::createSlicelet",
                    "src/DicerTestEnvironment.java",
                ),
                sources: BTreeSet::from([CandidateSource::TermIndex]),
                indexed_matches: BTreeSet::from([
                    "create".to_owned(),
                    "environment".to_owned(),
                    "slicelet".to_owned(),
                    "test".to_owned(),
                ]),
                relationship_matches: BTreeSet::new(),
            },
            SearchCandidate {
                node: owned_node(
                    "n:generic",
                    ".create()",
                    "com.example.Slicelet::create",
                    "src/Slicelet.java",
                ),
                sources: BTreeSet::from([CandidateSource::TermIndex]),
                indexed_matches: BTreeSet::from(["create".to_owned(), "slicelet".to_owned()]),
                relationship_matches: BTreeSet::new(),
            },
        ];

        let ranked = rank_search_candidates(
            "test environment create slicelet",
            &[
                "create".to_owned(),
                "environment".to_owned(),
                "slicelet".to_owned(),
                "test".to_owned(),
            ],
            candidates,
            usize::MAX,
        );

        assert_eq!(ranked[0].node_id, "n:environment");
    }

    #[test]
    fn generator_helper_does_not_beat_a_runtime_behavior() {
        let candidates = vec![
            SearchCandidate {
                node: owned_node(
                    "n:generator",
                    "save",
                    "Rails::Generators::ActiveModel::save",
                    "railties/lib/rails/generators/active_model.rb",
                ),
                sources: BTreeSet::from([CandidateSource::TermIndex]),
                indexed_matches: BTreeSet::from([
                    "active".to_owned(),
                    "model".to_owned(),
                    "save".to_owned(),
                ]),
                relationship_matches: BTreeSet::new(),
            },
            SearchCandidate {
                node: owned_node(
                    "n:runtime",
                    "save",
                    "ActiveRecord::Persistence::save",
                    "activerecord/lib/active_record/persistence.rb",
                ),
                sources: BTreeSet::from([CandidateSource::TermIndex]),
                indexed_matches: BTreeSet::from([
                    "active".to_owned(),
                    "record".to_owned(),
                    "save".to_owned(),
                ]),
                relationship_matches: BTreeSet::new(),
            },
        ];

        let ranked = rank_search_candidates(
            "how does Active Record persistence save a model",
            &[
                "active".to_owned(),
                "model".to_owned(),
                "persistence".to_owned(),
                "record".to_owned(),
                "save".to_owned(),
            ],
            candidates,
            usize::MAX,
        );

        assert_eq!(ranked[0].node_id, "n:runtime");
    }

    #[test]
    fn direct_behavior_fit_beats_equal_relationship_coverage() {
        let candidates = vec![
            SearchCandidate {
                node: node(
                    "n:direct",
                    "saveRepositoryState",
                    NodeKind::Method,
                    "src/state.rs",
                    false,
                ),
                sources: BTreeSet::from([CandidateSource::TermIndex]),
                indexed_matches: BTreeSet::from(["repository".to_owned(), "state".to_owned()]),
                relationship_matches: BTreeSet::new(),
            },
            SearchCandidate {
                node: node(
                    "n:relationship",
                    "save",
                    NodeKind::Method,
                    "src/workflow.rs",
                    false,
                ),
                sources: BTreeSet::from([CandidateSource::RelationSeed]),
                indexed_matches: BTreeSet::new(),
                relationship_matches: ["repository", "state"]
                    .into_iter()
                    .map(|term| RelationshipTermMatch {
                        term: term.to_owned(),
                        kind: EdgeKind::Calls,
                        target_ids: BTreeSet::from([format!("target:{term}")]),
                    })
                    .collect(),
            },
        ];

        let ranked = rank_search_candidates(
            "save repository state",
            &[
                "save".to_owned(),
                "repository".to_owned(),
                "state".to_owned(),
            ],
            candidates,
            usize::MAX,
        );

        assert_eq!(ranked[0].node_id, "n:direct");
    }

    #[test]
    fn source_backed_function_is_the_operation_root_for_a_terse_task_query() {
        let candidates = vec![
            SearchCandidate {
                node: node(
                    "n:node-type",
                    "Node",
                    NodeKind::Interface,
                    "src/types.ts",
                    false,
                ),
                sources: BTreeSet::from([CandidateSource::TermIndex]),
                indexed_matches: BTreeSet::from(["node".to_owned()]),
                relationship_matches: BTreeSet::new(),
            },
            SearchCandidate {
                node: node(
                    "n:bulk-load",
                    "beginBulkNodeLoad",
                    NodeKind::Function,
                    "src/db.ts",
                    false,
                ),
                sources: BTreeSet::from([CandidateSource::TermIndex]),
                indexed_matches: BTreeSet::from([
                    "bulk".to_owned(),
                    "load".to_owned(),
                    "node".to_owned(),
                ]),
                relationship_matches: BTreeSet::new(),
            },
        ];

        let ranked = rank_search_candidates(
            "enter bulk node load mode",
            &["bulk".to_owned(), "load".to_owned(), "node".to_owned()],
            candidates,
            usize::MAX,
        );

        assert_eq!(ranked[0].node_id, "n:bulk-load");
        assert_eq!(ranked[0].channel_rank, 4);
    }

    #[test]
    fn common_library_operation_words_align_with_symbol_predicates() {
        for (query, symbol, canonical) in [
            ("construct", "build", "create"),
            ("decode", "decode", "decode"),
            ("execute", "handle", "execute"),
            ("extract", "extract", "extract"),
            ("iterate", "iter", "iterate"),
            ("instantiate", "create", "create"),
            ("parse", "parse", "parse"),
            ("raise", "raise", "raise"),
            ("select", "get", "read"),
            ("send", "send", "send"),
        ] {
            assert_eq!(canonical_predicate_token(query), Some(canonical));
            assert_eq!(canonical_predicate_token(symbol), Some(canonical));
            assert!(is_explicit_operation_predicate(query));
            assert!(is_explicit_operation_predicate(symbol));
        }
    }

    #[test]
    fn owner_initialisms_distinguish_protocol_types_from_ordinary_compound_types() {
        assert!(has_owner_initialism("WSGITransport"));
        assert!(has_owner_initialism("HTTPAdapter"));
        assert!(!has_owner_initialism("URL"));
        assert!(!has_owner_initialism("ModuleCompiler"));
        assert!(!has_owner_initialism("ClientProxy"));
    }

    #[test]
    fn execution_query_prefers_the_framework_transport_handler() {
        let candidates = vec![
            SearchCandidate {
                node: owned_node(
                    "n:generic",
                    ".executeRequest()",
                    "httpx._client.Client::executeRequest",
                    "httpx/_client.py",
                ),
                sources: BTreeSet::from([CandidateSource::TermIndex]),
                indexed_matches: BTreeSet::from(["execute".to_owned(), "request".to_owned()]),
                relationship_matches: BTreeSet::new(),
            },
            SearchCandidate {
                node: owned_node(
                    "n:wsgi",
                    ".handle_request()",
                    "httpx._transports.wsgi.WSGITransport::handle_request",
                    "httpx/_transports/wsgi.py",
                ),
                sources: BTreeSet::from([CandidateSource::TermIndex]),
                indexed_matches: BTreeSet::from(["request".to_owned(), "wsgi".to_owned()]),
                relationship_matches: BTreeSet::new(),
            },
        ];

        let ranked = rank_search_candidates(
            "execute request through WSGI application",
            &[
                "application".to_owned(),
                "execute".to_owned(),
                "request".to_owned(),
                "wsgi".to_owned(),
            ],
            candidates,
            usize::MAX,
        );

        assert_eq!(ranked[0].node_id, "n:wsgi");
    }

    #[test]
    fn aspnet_queries_prefer_request_and_middleware_execution_anchors() {
        let mut request = owned_node(
            "n:http-protocol",
            ".ProcessRequestsAsync()",
            "Microsoft.AspNetCore.Server.Kestrel.Core.Internal.Http.HttpProtocol::ProcessRequestsAsync",
            "src/Servers/Kestrel/Core/src/Internal/Http/HttpProtocol.cs",
        );
        request.language = Some("csharp".to_owned());
        let request_candidates = vec![
            SearchCandidate {
                node: request,
                sources: BTreeSet::from([CandidateSource::TermIndex]),
                indexed_matches: BTreeSet::from([
                    "http".to_owned(),
                    "process".to_owned(),
                    "request".to_owned(),
                ]),
                relationship_matches: BTreeSet::new(),
            },
            SearchCandidate {
                node: node(
                    "n:process-manager",
                    "PROCESS_MANAGER",
                    NodeKind::Class,
                    "src/Servers/IIS/OutOfProcessRequestHandler/processmanager.h",
                    false,
                ),
                sources: BTreeSet::from([CandidateSource::TermIndex]),
                indexed_matches: BTreeSet::from(["process".to_owned()]),
                relationship_matches: BTreeSet::new(),
            },
        ];
        let ranked = rank_search_candidates(
            "how are HTTP requests processed",
            &[
                "http".to_owned(),
                "process".to_owned(),
                "request".to_owned(),
            ],
            request_candidates,
            usize::MAX,
        );
        assert_eq!(ranked[0].node_id, "n:http-protocol", "ranked={ranked:#?}");

        let mut middleware = owned_node(
            "n:middleware-invoke",
            ".InvokeAsync()",
            "Microsoft.AspNetCore.Authentication.AuthenticationMiddleware::InvokeAsync",
            "src/Security/Authentication/Core/src/AuthenticationMiddleware.cs",
        );
        middleware.language = Some("csharp".to_owned());
        let ranked = rank_search_candidates(
            "how is middleware invoked",
            &["invoke".to_owned(), "middleware".to_owned()],
            vec![
                SearchCandidate {
                    node: middleware,
                    sources: BTreeSet::from([CandidateSource::TermIndex]),
                    indexed_matches: BTreeSet::from(["invoke".to_owned(), "middleware".to_owned()]),
                    relationship_matches: BTreeSet::new(),
                },
                SearchCandidate {
                    node: owned_node(
                        "n:middleware-factory",
                        ".Create()",
                        "Microsoft.AspNetCore.Http.MiddlewareFactory::Create",
                        "src/Http/Http/src/MiddlewareFactory.cs",
                    ),
                    sources: BTreeSet::from([CandidateSource::TermIndex]),
                    indexed_matches: BTreeSet::from(["middleware".to_owned()]),
                    relationship_matches: BTreeSet::new(),
                },
            ],
            usize::MAX,
        );
        assert_eq!(ranked[0].node_id, "n:middleware-factory");
    }

    #[test]
    fn decoder_chain_query_prefers_the_multi_decoder_owner() {
        let candidates = [
            ("n:content", "httpx._decoders.ContentDecoder::decode"),
            ("n:multi", "httpx._decoders.MultiDecoder::decode"),
        ]
        .into_iter()
        .map(|(id, qualified_name)| SearchCandidate {
            node: owned_node(id, ".decode()", qualified_name, "httpx/_decoders.py"),
            sources: BTreeSet::from([CandidateSource::Alias]),
            indexed_matches: BTreeSet::from(["decode".to_owned(), "decoder".to_owned()]),
            relationship_matches: BTreeSet::new(),
        })
        .collect();

        let ranked = rank_search_candidates(
            "decode compressed response through decoder chain",
            &[
                "chain".to_owned(),
                "compress".to_owned(),
                "decode".to_owned(),
                "decoder".to_owned(),
                "response".to_owned(),
            ],
            candidates,
            usize::MAX,
        );

        assert_eq!(ranked[0].node_id, "n:multi");
    }

    #[test]
    fn compound_owner_does_not_promote_an_unrelated_rust_method() {
        let candidates = vec![
            SearchCandidate {
                node: owned_node(
                    "n:storage",
                    ".build_storage()",
                    "deltalake_core::table::builder::DeltaTableBuilder::build_storage",
                    "crates/core/src/table/builder.rs",
                ),
                sources: BTreeSet::from([CandidateSource::TermIndex]),
                indexed_matches: BTreeSet::from(["delta".to_owned(), "table".to_owned()]),
                relationship_matches: BTreeSet::new(),
            },
            SearchCandidate {
                node: node(
                    "n:builder",
                    "DeltaTableBuilder",
                    NodeKind::Struct,
                    "crates/core/src/table/builder.rs",
                    false,
                ),
                sources: BTreeSet::from([CandidateSource::TermIndex]),
                indexed_matches: BTreeSet::from(["delta".to_owned(), "table".to_owned()]),
                relationship_matches: BTreeSet::new(),
            },
        ];

        let ranked = rank_search_candidates(
            "how is a Delta table opened from a URL",
            &["delta".to_owned(), "open".to_owned(), "table".to_owned()],
            candidates,
            usize::MAX,
        );

        assert_eq!(ranked[0].node_id, "n:builder");
    }

    #[test]
    fn complete_function_name_beats_partial_contextual_symbols() {
        let candidates = vec![
            SearchCandidate {
                node: node(
                    "n:ignore-method",
                    ".ignores()",
                    NodeKind::Method,
                    "src/extraction.ts",
                    false,
                ),
                sources: BTreeSet::from([CandidateSource::TermIndex]),
                indexed_matches: BTreeSet::from(["ignore".to_owned()]),
                relationship_matches: BTreeSet::new(),
            },
            SearchCandidate {
                node: node(
                    "n:defaults-only",
                    "defaultsOnlyIgnore",
                    NodeKind::Function,
                    "src/extraction.ts",
                    false,
                ),
                sources: BTreeSet::from([CandidateSource::TermIndex]),
                indexed_matches: BTreeSet::from([
                    "default".to_owned(),
                    "ignore".to_owned(),
                    "only".to_owned(),
                ]),
                relationship_matches: BTreeSet::new(),
            },
        ];

        let ranked = rank_search_candidates(
            "defaults-only ignore matcher for embedded repositories",
            &[
                "default".to_owned(),
                "embedded".to_owned(),
                "ignore".to_owned(),
                "matcher".to_owned(),
                "only".to_owned(),
                "repository".to_owned(),
            ],
            candidates,
            usize::MAX,
        );

        assert_eq!(ranked[0].node_id, "n:defaults-only");
    }

    #[test]
    fn complete_method_name_beats_a_single_matching_type_term() {
        let mut method = node(
            "n:prefix",
            ".getNodesByNamePrefix()",
            NodeKind::Method,
            "db/queries.ts",
            false,
        );
        method.qualified_name = "queries.QueryBuilder.getNodesByNamePrefix".to_owned();
        let candidates = vec![
            SearchCandidate {
                node: node(
                    "n:node-type",
                    "Node",
                    NodeKind::Interface,
                    "types.ts",
                    false,
                ),
                sources: BTreeSet::from([CandidateSource::TermIndex]),
                indexed_matches: BTreeSet::from(["node".to_owned()]),
                relationship_matches: BTreeSet::new(),
            },
            SearchCandidate {
                node: method,
                sources: BTreeSet::from([CandidateSource::TermIndex]),
                indexed_matches: BTreeSet::from([
                    "by".to_owned(),
                    "name".to_owned(),
                    "node".to_owned(),
                    "prefix".to_owned(),
                ]),
                relationship_matches: BTreeSet::new(),
            },
        ];

        let ranked = rank_search_candidates(
            "nodes by name prefix",
            &["name".to_owned(), "node".to_owned(), "prefix".to_owned()],
            candidates,
            usize::MAX,
        );

        assert_eq!(ranked[0].node_id, "n:prefix");
    }

    #[test]
    fn matched_method_subject_width_ignores_its_operation_predicate() {
        let mut method = node(
            "n:incoming",
            ".getIncomingEdges()",
            NodeKind::Method,
            "db/queries.ts",
            false,
        );
        method.qualified_name = "queries.QueryBuilder.getIncomingEdges".to_owned();
        let candidates = vec![
            SearchCandidate {
                node: node("n:edge-kind", "EdgeKind", NodeKind::Enum, "types.ts", false),
                sources: BTreeSet::from([CandidateSource::TermIndex]),
                indexed_matches: BTreeSet::from(["edge".to_owned(), "kind".to_owned()]),
                relationship_matches: BTreeSet::new(),
            },
            SearchCandidate {
                node: method,
                sources: BTreeSet::from([CandidateSource::TermIndex]),
                indexed_matches: BTreeSet::from(["edge".to_owned(), "incoming".to_owned()]),
                relationship_matches: BTreeSet::new(),
            },
        ];

        let ranked = rank_search_candidates(
            "incoming edges",
            &["edge".to_owned(), "incoming".to_owned()],
            candidates,
            usize::MAX,
        );

        assert_eq!(ranked[0].node_id, "n:incoming");
    }

    #[test]
    fn source_backed_builder_is_the_operation_root_for_natural_questions() {
        let candidates = vec![
            SearchCandidate {
                node: node(
                    "n:generic",
                    "writeDeltaTableData",
                    NodeKind::Function,
                    "src/operations.rs",
                    false,
                ),
                sources: BTreeSet::from([CandidateSource::TermIndex]),
                indexed_matches: BTreeSet::from([
                    "data".to_owned(),
                    "delta".to_owned(),
                    "table".to_owned(),
                    "write".to_owned(),
                ]),
                relationship_matches: BTreeSet::new(),
            },
            SearchCandidate {
                node: node(
                    "n:builder",
                    "WriteBuilder",
                    NodeKind::Struct,
                    "src/operations/write.rs",
                    false,
                ),
                sources: BTreeSet::from([CandidateSource::TermIndex]),
                indexed_matches: BTreeSet::from(["write".to_owned()]),
                relationship_matches: BTreeSet::new(),
            },
        ];

        let ranked = rank_search_candidates(
            "how does delta write data to a table",
            &[
                "data".to_owned(),
                "delta".to_owned(),
                "table".to_owned(),
                "write".to_owned(),
            ],
            candidates,
            usize::MAX,
        );

        assert_eq!(ranked[0].node_id, "n:builder");
        assert_eq!(ranked[0].channel_rank, 4);
    }

    #[test]
    fn direct_predicate_root_beats_a_subject_handler_with_a_noun_synonym() {
        let candidates = vec![
            SearchCandidate {
                node: node(
                    "n:storage-handler",
                    "DeltaStorageHandler",
                    NodeKind::Class,
                    "python/fs_handler.py",
                    false,
                ),
                sources: BTreeSet::from([CandidateSource::TermIndex]),
                indexed_matches: BTreeSet::from(["delta".to_owned(), "write".to_owned()]),
                relationship_matches: BTreeSet::new(),
            },
            SearchCandidate {
                node: node(
                    "n:write-builder",
                    "WriteBuilder",
                    NodeKind::Struct,
                    "src/operations/write.rs",
                    false,
                ),
                sources: BTreeSet::from([CandidateSource::TermIndex]),
                indexed_matches: BTreeSet::from(["write".to_owned()]),
                relationship_matches: BTreeSet::new(),
            },
        ];

        let ranked = rank_search_candidates(
            "how does delta write data to a table",
            &[
                "data".to_owned(),
                "delta".to_owned(),
                "table".to_owned(),
                "write".to_owned(),
            ],
            candidates,
            usize::MAX,
        );

        assert_eq!(ranked[0].node_id, "n:write-builder");
    }

    #[test]
    fn source_less_builder_does_not_claim_operation_root_evidence() {
        let candidates = vec![SearchCandidate {
            node: node(
                "n:synthetic-builder",
                "WriteBuilder",
                NodeKind::Struct,
                "",
                false,
            ),
            sources: BTreeSet::from([CandidateSource::TermIndex]),
            indexed_matches: BTreeSet::from(["write".to_owned()]),
            relationship_matches: BTreeSet::new(),
        }];

        let ranked = rank_search_candidates(
            "how is data written",
            &["data".to_owned(), "write".to_owned()],
            candidates,
            usize::MAX,
        );

        assert_ne!(ranked[0].channel_rank, 4);
        assert!(ranked[0].operation_root.is_none());
    }

    #[test]
    fn predicate_synonyms_rank_but_do_not_create_operation_root_eligibility() {
        let candidates = vec![SearchCandidate {
            node: node(
                "n:write-builder",
                "WriteBuilder",
                NodeKind::Struct,
                "src/write.rs",
                false,
            ),
            sources: BTreeSet::from([CandidateSource::Fuzzy]),
            indexed_matches: BTreeSet::new(),
            relationship_matches: BTreeSet::new(),
        }];

        let ranked = rank_search_candidates(
            "how are records saved",
            &["record".to_owned(), "save".to_owned()],
            candidates,
            usize::MAX,
        );

        assert!(ranked[0].operation_root.is_none());
        assert_ne!(ranked[0].channel_rank, 4);
    }

    #[test]
    fn exact_single_token_type_is_a_representation_root() {
        let candidates = vec![
            SearchCandidate {
                node: node(
                    "n:method",
                    "tableSnapshot",
                    NodeKind::Method,
                    "src/table.rs",
                    false,
                ),
                sources: BTreeSet::from([CandidateSource::TermIndex]),
                indexed_matches: BTreeSet::from(["snapshot".to_owned(), "table".to_owned()]),
                relationship_matches: BTreeSet::new(),
            },
            SearchCandidate {
                node: node(
                    "n:snapshot",
                    "Snapshot",
                    NodeKind::Struct,
                    "src/snapshot.rs",
                    false,
                ),
                sources: BTreeSet::from([CandidateSource::TermIndex]),
                indexed_matches: BTreeSet::from(["snapshot".to_owned()]),
                relationship_matches: BTreeSet::new(),
            },
        ];

        let ranked = rank_search_candidates(
            "where is a table snapshot represented",
            &["snapshot".to_owned(), "table".to_owned()],
            candidates,
            usize::MAX,
        );

        assert_eq!(ranked[0].node_id, "n:snapshot");
    }

    #[test]
    fn complete_subject_coverage_precedes_a_generic_mutator_predicate() {
        let candidates = vec![
            SearchCandidate {
                node: node(
                    "n:add-column",
                    "AddColumnBuilder",
                    NodeKind::Struct,
                    "src/add_column.rs",
                    false,
                ),
                sources: BTreeSet::from([CandidateSource::TermIndex]),
                indexed_matches: BTreeSet::from(["add".to_owned()]),
                relationship_matches: BTreeSet::new(),
            },
            SearchCandidate {
                node: node(
                    "n:constraint",
                    "ConstraintBuilder",
                    NodeKind::Struct,
                    "src/constraints.rs",
                    false,
                ),
                sources: BTreeSet::from([CandidateSource::TermIndex]),
                indexed_matches: BTreeSet::from(["constraint".to_owned()]),
                relationship_matches: BTreeSet::new(),
            },
        ];

        let ranked = rank_search_candidates(
            "how are table constraints added",
            &[
                "add".to_owned(),
                "constraint".to_owned(),
                "table".to_owned(),
            ],
            candidates,
            usize::MAX,
        );

        assert_eq!(ranked[0].node_id, "n:constraint");
    }

    #[test]
    fn precise_operation_subject_beats_a_broader_type_with_the_same_predicate() {
        let candidates = vec![
            SearchCandidate {
                node: node(
                    "n:broad-update",
                    "UpdateTableMetadataBuilder",
                    NodeKind::Struct,
                    "src/table.rs",
                    false,
                ),
                sources: BTreeSet::from([CandidateSource::TermIndex]),
                indexed_matches: BTreeSet::from(["table".to_owned(), "update".to_owned()]),
                relationship_matches: BTreeSet::new(),
            },
            SearchCandidate {
                node: node(
                    "n:update",
                    "UpdateBuilder",
                    NodeKind::Struct,
                    "src/update.rs",
                    false,
                ),
                sources: BTreeSet::from([CandidateSource::TermIndex]),
                indexed_matches: BTreeSet::from(["update".to_owned()]),
                relationship_matches: BTreeSet::new(),
            },
        ];

        let ranked = rank_search_candidates(
            "how are rows updated in a table",
            &["row".to_owned(), "table".to_owned(), "update".to_owned()],
            candidates,
            usize::MAX,
        );

        assert_eq!(ranked[0].node_id, "n:update");
    }

    #[test]
    fn complete_multi_concept_representation_beats_a_role_type() {
        let candidates = vec![
            SearchCandidate {
                node: node(
                    "n:builder",
                    "DeltaTableBuilder",
                    NodeKind::Struct,
                    "src/builder.rs",
                    false,
                ),
                sources: BTreeSet::from([CandidateSource::TermIndex]),
                indexed_matches: BTreeSet::from(["delta".to_owned(), "table".to_owned()]),
                relationship_matches: BTreeSet::new(),
            },
            SearchCandidate {
                node: node(
                    "n:state",
                    "DeltaTableState",
                    NodeKind::Struct,
                    "src/state.rs",
                    false,
                ),
                sources: BTreeSet::from([CandidateSource::TermIndex]),
                indexed_matches: BTreeSet::from([
                    "delta".to_owned(),
                    "state".to_owned(),
                    "table".to_owned(),
                ]),
                relationship_matches: BTreeSet::new(),
            },
        ];

        let ranked = rank_search_candidates(
            "where is delta table state represented",
            &[
                "delta".to_owned(),
                "represent".to_owned(),
                "state".to_owned(),
                "table".to_owned(),
            ],
            candidates,
            usize::MAX,
        );

        assert_eq!(ranked[0].node_id, "n:state");
    }

    #[test]
    fn action_specific_root_beats_the_matching_representation_type() {
        let candidates = vec![
            SearchCandidate {
                node: node(
                    "n:properties",
                    "TableProperties",
                    NodeKind::Struct,
                    "src/properties.rs",
                    false,
                ),
                sources: BTreeSet::from([CandidateSource::TermIndex]),
                indexed_matches: BTreeSet::from(["properties".to_owned(), "table".to_owned()]),
                relationship_matches: BTreeSet::new(),
            },
            SearchCandidate {
                node: node(
                    "n:set-properties",
                    "SetTablePropertiesBuilder",
                    NodeKind::Struct,
                    "src/set_properties.rs",
                    false,
                ),
                sources: BTreeSet::from([CandidateSource::TermIndex]),
                indexed_matches: BTreeSet::from(["properties".to_owned(), "table".to_owned()]),
                relationship_matches: BTreeSet::new(),
            },
        ];

        let ranked = rank_search_candidates(
            "how are table properties changed",
            &[
                "change".to_owned(),
                "properties".to_owned(),
                "table".to_owned(),
            ],
            candidates,
            usize::MAX,
        );

        assert_eq!(ranked[0].node_id, "n:set-properties");
    }

    #[test]
    fn complete_transaction_subject_beats_an_unrelated_configuration_root() {
        let mut commit_properties = node(
            "n:commit-properties",
            "CommitProperties",
            NodeKind::Struct,
            "src/transaction.rs",
            false,
        );
        commit_properties.qualified_name = "transaction::CommitProperties".to_owned();
        let candidates = vec![
            SearchCandidate {
                node: commit_properties,
                sources: BTreeSet::from([CandidateSource::TermIndex]),
                indexed_matches: BTreeSet::from([
                    "commit".to_owned(),
                    "properties".to_owned(),
                    "transaction".to_owned(),
                ]),
                relationship_matches: BTreeSet::new(),
            },
            SearchCandidate {
                node: node(
                    "n:set-properties",
                    "SetTablePropertiesBuilder",
                    NodeKind::Struct,
                    "src/set_properties.rs",
                    false,
                ),
                sources: BTreeSet::from([CandidateSource::TermIndex]),
                indexed_matches: BTreeSet::from(["configure".to_owned(), "properties".to_owned()]),
                relationship_matches: BTreeSet::new(),
            },
        ];

        let ranked = rank_search_candidates(
            "where are transaction commit properties configured",
            &[
                "commit".to_owned(),
                "configure".to_owned(),
                "properties".to_owned(),
                "transaction".to_owned(),
            ],
            candidates,
            usize::MAX,
        );

        assert_eq!(ranked[0].node_id, "n:commit-properties");
    }

    #[test]
    fn predicate_and_subject_alignment_beats_a_generic_full_subject_role() {
        let candidates = vec![
            SearchCandidate {
                node: node(
                    "n:generic",
                    "DeltaTableFactory",
                    NodeKind::Struct,
                    "src/table.rs",
                    false,
                ),
                sources: BTreeSet::from([CandidateSource::TermIndex]),
                indexed_matches: BTreeSet::from(["delta".to_owned(), "table".to_owned()]),
                relationship_matches: BTreeSet::new(),
            },
            SearchCandidate {
                node: node(
                    "n:convert",
                    "ConvertToDeltaBuilder",
                    NodeKind::Struct,
                    "src/convert.rs",
                    false,
                ),
                sources: BTreeSet::from([CandidateSource::TermIndex]),
                indexed_matches: BTreeSet::from(["convert".to_owned(), "delta".to_owned()]),
                relationship_matches: BTreeSet::new(),
            },
        ];

        let ranked = rank_search_candidates(
            "how are files converted into a delta table",
            &[
                "convert".to_owned(),
                "delta".to_owned(),
                "file".to_owned(),
                "table".to_owned(),
            ],
            candidates,
            usize::MAX,
        );

        assert_eq!(ranked[0].node_id, "n:convert");
    }

    #[test]
    fn storage_noun_does_not_promote_a_factory_over_the_abstraction() {
        let candidates = vec![
            SearchCandidate {
                node: node(
                    "n:store",
                    "LogStore",
                    NodeKind::Trait,
                    "src/logstore.rs",
                    false,
                ),
                sources: BTreeSet::from([CandidateSource::TermIndex]),
                indexed_matches: BTreeSet::from(["log".to_owned(), "storage".to_owned()]),
                relationship_matches: BTreeSet::new(),
            },
            SearchCandidate {
                node: node(
                    "n:factory",
                    "LogStoreFactory",
                    NodeKind::Trait,
                    "src/logstore.rs",
                    false,
                ),
                sources: BTreeSet::from([CandidateSource::TermIndex]),
                indexed_matches: BTreeSet::from(["log".to_owned(), "storage".to_owned()]),
                relationship_matches: BTreeSet::new(),
            },
        ];

        let ranked = rank_search_candidates(
            "how are transaction log storage backends abstracted",
            &[
                "abstract".to_owned(),
                "backend".to_owned(),
                "log".to_owned(),
                "storage".to_owned(),
                "transaction".to_owned(),
            ],
            candidates,
            usize::MAX,
        );

        assert_eq!(ranked[0].node_id, "n:store");
    }

    #[test]
    fn phrase_initialism_selects_the_specific_operation_root() {
        let candidates = vec![
            SearchCandidate {
                node: node(
                    "n:load",
                    "LoadBuilder",
                    NodeKind::Struct,
                    "src/load.rs",
                    false,
                ),
                sources: BTreeSet::from([CandidateSource::TermIndex]),
                indexed_matches: BTreeSet::from(["load".to_owned()]),
                relationship_matches: BTreeSet::new(),
            },
            SearchCandidate {
                node: node(
                    "n:cdf-load",
                    "CdfLoadBuilder",
                    NodeKind::Struct,
                    "src/load_cdf.rs",
                    false,
                ),
                sources: BTreeSet::from([CandidateSource::TermIndex]),
                indexed_matches: BTreeSet::from(["load".to_owned()]),
                relationship_matches: BTreeSet::new(),
            },
        ];

        let ranked = rank_search_candidates(
            "how is change data feed loaded",
            &[
                "change".to_owned(),
                "data".to_owned(),
                "feed".to_owned(),
                "load".to_owned(),
            ],
            candidates,
            usize::MAX,
        );

        assert_eq!(ranked[0].node_id, "n:cdf-load");
    }

    #[test]
    fn uncovered_persistence_predicate_beats_repeated_entity_terms_and_broad_fanout() {
        let relation = |targets: &[&str]| {
            ["repository", "state"]
                .into_iter()
                .map(|term| RelationshipTermMatch {
                    term: term.to_owned(),
                    kind: EdgeKind::Calls,
                    target_ids: targets.iter().map(|target| (*target).to_owned()).collect(),
                })
                .collect::<BTreeSet<_>>()
        };
        let candidates = vec![
            SearchCandidate {
                node: node(
                    "n:lifecycle",
                    "RepositoryStateManager",
                    NodeKind::Function,
                    "src/lifecycle.go",
                    false,
                ),
                sources: BTreeSet::from([CandidateSource::RelationSeed]),
                indexed_matches: BTreeSet::new(),
                relationship_matches: relation(&["t:repository", "t:state:1", "t:state:2"]),
            },
            SearchCandidate {
                node: node(
                    "n:save-step",
                    "SaveStep",
                    NodeKind::Method,
                    "src/manual_commit_git.go",
                    false,
                ),
                sources: BTreeSet::from([CandidateSource::RelationSeed]),
                indexed_matches: BTreeSet::new(),
                relationship_matches: relation(&["t:repository", "t:state:1"]),
            },
        ];

        let ranked = rank_search_candidates(
            "how is repository state recorded",
            &[
                "record".to_owned(),
                "repository".to_owned(),
                "state".to_owned(),
            ],
            candidates,
            usize::MAX,
        );

        assert_eq!(ranked[0].node_id, "n:save-step");
        assert_eq!(ranked[0].candidate_source, CandidateSource::RelationSeed);
        assert!(ranked[0].relation_evidence > ranked[1].relation_evidence);
    }

    #[test]
    fn persistence_predicates_are_whole_tokens_not_substrings() {
        let relation = ["repository", "state"]
            .into_iter()
            .map(|term| RelationshipTermMatch {
                term: term.to_owned(),
                kind: EdgeKind::Calls,
                target_ids: BTreeSet::from([format!("target:{term}")]),
            })
            .collect::<BTreeSet<_>>();
        let candidates = vec![
            SearchCandidate {
                node: node(
                    "n:rewrite",
                    "RewriteStep",
                    NodeKind::Method,
                    "src/rewrite.rs",
                    false,
                ),
                sources: BTreeSet::from([CandidateSource::RelationSeed]),
                indexed_matches: BTreeSet::new(),
                relationship_matches: relation.clone(),
            },
            SearchCandidate {
                node: node(
                    "n:write",
                    "WriteStep",
                    NodeKind::Method,
                    "src/write.rs",
                    false,
                ),
                sources: BTreeSet::from([CandidateSource::RelationSeed]),
                indexed_matches: BTreeSet::new(),
                relationship_matches: relation,
            },
        ];

        let ranked = rank_search_candidates(
            "how is repository state written",
            &[
                "repository".to_owned(),
                "state".to_owned(),
                "written".to_owned(),
            ],
            candidates,
            usize::MAX,
        );

        assert_eq!(ranked[0].node_id, "n:write");
        assert!(ranked[0].relation_evidence > ranked[1].relation_evidence);
    }

    #[test]
    fn equally_aligned_relation_candidates_retain_equal_ambiguity_evidence() {
        let relation = ["repository", "state"]
            .into_iter()
            .map(|term| RelationshipTermMatch {
                term: term.to_owned(),
                kind: EdgeKind::Calls,
                target_ids: BTreeSet::from([format!("target:{term}")]),
            })
            .collect::<BTreeSet<_>>();
        let candidates = ["n:first", "n:second"]
            .into_iter()
            .map(|id| SearchCandidate {
                node: node(id, "SaveStep", NodeKind::Method, "src/step.rs", false),
                sources: BTreeSet::from([CandidateSource::RelationSeed]),
                indexed_matches: BTreeSet::new(),
                relationship_matches: relation.clone(),
            })
            .collect::<Vec<_>>();

        let ranked = rank_search_candidates(
            "how is repository state recorded",
            &[
                "record".to_owned(),
                "repository".to_owned(),
                "state".to_owned(),
            ],
            candidates,
            usize::MAX,
        );

        assert_eq!(ranked[0].relation_evidence, ranked[1].relation_evidence);
    }

    #[test]
    fn relation_predicate_precision_precedes_support_count() {
        let relation = |targets: &[&str]| {
            ["repository", "state"]
                .into_iter()
                .map(|term| RelationshipTermMatch {
                    term: term.to_owned(),
                    kind: EdgeKind::Calls,
                    target_ids: targets.iter().map(|target| (*target).to_owned()).collect(),
                })
                .collect::<BTreeSet<_>>()
        };
        let candidates = vec![
            SearchCandidate {
                node: node(
                    "n:less-precise",
                    "SaveTaskStep",
                    NodeKind::Method,
                    "src/task.rs",
                    false,
                ),
                sources: BTreeSet::from([CandidateSource::RelationSeed]),
                indexed_matches: BTreeSet::new(),
                relationship_matches: relation(&["t:repository", "t:state:1", "t:state:2"]),
            },
            SearchCandidate {
                node: node(
                    "n:precise",
                    "SaveStep",
                    NodeKind::Method,
                    "src/step.rs",
                    false,
                ),
                sources: BTreeSet::from([CandidateSource::RelationSeed]),
                indexed_matches: BTreeSet::new(),
                relationship_matches: relation(&["t:repository", "t:state:1"]),
            },
        ];

        let ranked = rank_search_candidates(
            "how is repository state recorded",
            &[
                "record".to_owned(),
                "repository".to_owned(),
                "state".to_owned(),
            ],
            candidates,
            usize::MAX,
        );

        assert_eq!(ranked[0].node_id, "n:precise");
    }

    #[test]
    fn persistence_vocabulary_alone_does_not_create_relation_eligibility() {
        let candidates = vec![SearchCandidate {
            node: node(
                "n:save-helper",
                "SaveStep",
                NodeKind::Method,
                "src/helper.rs",
                false,
            ),
            sources: BTreeSet::from([CandidateSource::TermIndex]),
            indexed_matches: BTreeSet::from(["save".to_owned()]),
            relationship_matches: BTreeSet::new(),
        }];

        let ranked = rank_search_candidates(
            "how is repository state recorded",
            &[
                "record".to_owned(),
                "repository".to_owned(),
                "state".to_owned(),
            ],
            candidates,
            usize::MAX,
        );

        assert_eq!(ranked[0].candidate_source, CandidateSource::TermIndex);
        assert!(ranked[0].relation_evidence.is_none());
        assert_ne!(ranked[0].channel_rank, 4);
    }

    #[test]
    fn uncovered_nouns_are_not_treated_as_operation_predicates() {
        let relationship_matches = ["repository", "state"]
            .into_iter()
            .map(|term| RelationshipTermMatch {
                term: term.to_owned(),
                kind: EdgeKind::Calls,
                target_ids: BTreeSet::from([format!("target:{term}")]),
            })
            .collect();
        let candidates = vec![SearchCandidate {
            node: node(
                "n:lifecycle",
                "LifecycleManager",
                NodeKind::Function,
                "src/lifecycle.rs",
                false,
            ),
            sources: BTreeSet::from([CandidateSource::RelationSeed]),
            indexed_matches: BTreeSet::new(),
            relationship_matches,
        }];

        let ranked = rank_search_candidates(
            "repository state lifecycle",
            &[
                "repository".to_owned(),
                "state".to_owned(),
                "lifecycle".to_owned(),
            ],
            candidates,
            usize::MAX,
        );

        assert_eq!(
            ranked[0]
                .relation_evidence
                .map(|evidence| evidence.predicate_match_count),
            Some(0)
        );
    }

    #[test]
    fn profile_v2_tiebreaks_stably_for_equal_scores() {
        let candidates = vec![
            SearchCandidate {
                node: node("n:aa", "same", NodeKind::Function, "src/lib.rs", false),
                sources: BTreeSet::from([CandidateSource::Alias]),
                indexed_matches: BTreeSet::new(),
                relationship_matches: BTreeSet::new(),
            },
            SearchCandidate {
                node: node("n:ab", "same", NodeKind::Function, "src/lib.rs", false),
                sources: BTreeSet::from([CandidateSource::Alias]),
                indexed_matches: BTreeSet::new(),
                relationship_matches: BTreeSet::new(),
            },
        ];
        let ranked = rank_search_candidates(
            "same",
            std::slice::from_ref(&"same".to_owned()),
            candidates,
            usize::MAX,
        );
        assert_eq!(ranked[0].node_id, "n:aa");
        assert_eq!(ranked[1].node_id, "n:ab");
    }

    #[test]
    fn bounded_ranking_matches_the_prefix_of_a_full_ranking() {
        let candidates = ["n:d", "n:b", "n:a", "n:c"]
            .into_iter()
            .map(|id| SearchCandidate {
                node: node(id, "same", NodeKind::Function, "src/lib.rs", false),
                sources: BTreeSet::from([CandidateSource::Alias]),
                indexed_matches: BTreeSet::new(),
                relationship_matches: BTreeSet::new(),
            })
            .collect::<Vec<_>>();
        let full = rank_search_candidates(
            "same",
            std::slice::from_ref(&"same".to_owned()),
            candidates.clone(),
            usize::MAX,
        );
        let bounded = rank_search_candidates(
            "same",
            std::slice::from_ref(&"same".to_owned()),
            candidates,
            2,
        );
        assert_eq!(
            bounded
                .iter()
                .map(|result| result.node_id.as_str())
                .collect::<Vec<_>>(),
            full.iter()
                .take(2)
                .map(|result| result.node_id.as_str())
                .collect::<Vec<_>>()
        );
    }

    #[test]
    fn owner_terms_beat_unrelated_methods_with_the_same_behavior_across_languages() {
        for (question, expected, distractor) in [
            (
                "how does a model save data",
                owned_node(
                    "n:model-save",
                    ".save()",
                    "django.db.models.base.Model::save",
                    "django/db/models/base.py",
                ),
                owned_node(
                    "n:file-save",
                    ".save()",
                    "django.db.models.fields.files.FieldFile::save",
                    "django/db/models/fields/files.py",
                ),
            ),
            (
                "how does the service container resolve bindings",
                owned_node(
                    "n:container-resolve",
                    ".resolve()",
                    "Container::resolve",
                    "src/Container/Container.php",
                ),
                owned_node(
                    "n:authenticated-resolve",
                    ".resolve()",
                    "Authenticated::resolve",
                    "src/Container/Attributes/Authenticated.php",
                ),
            ),
            (
                "how does the world add components",
                owned_node(
                    "n:world-add",
                    "add",
                    "bevy::ecs::world::World::add",
                    "crates/bevy_ecs/src/world/mod.rs",
                ),
                owned_node(
                    "n:ecs-add",
                    "add",
                    "bevy::world::ecs::Commands::add",
                    "crates/bevy_ecs/src/system/commands.rs",
                ),
            ),
            (
                "Claude generator generate summary",
                owned_node(
                    "n:claude-generate",
                    ".Generate()",
                    "cmd/entire/cli/summarize.ClaudeGenerator::Generate",
                    "cmd/entire/cli/summarize/claude.go",
                ),
                owned_node(
                    "n:generic-generate-summary",
                    "generateSummary()",
                    "cmd/entire/cli/strategy.generateSummary",
                    "cmd/entire/cli/strategy/manual_commit_condensation.go",
                ),
            ),
            (
                "create router execution context",
                owned_node(
                    "n:router-context-create",
                    ".create()",
                    "router-execution-context.RouterExecutionContext.create",
                    "packages/core/router/router-execution-context.ts",
                ),
                owned_node(
                    "n:router-explorer-create",
                    ".create()",
                    "router-explorer.RouterExplorer.create",
                    "packages/core/router/router-explorer.ts",
                ),
            ),
            (
                "convert scala slice key",
                owned_node(
                    "n:convert-scala-slice-key",
                    ".convertToScalaSliceKey()",
                    "com.databricks.dicer.external.javaapi.ImplFriend::convertToScalaSliceKey",
                    "src/main/java/com/databricks/dicer/external/javaapi/ImplFriend.java",
                ),
                owned_node(
                    "n:slice-key-to-scala",
                    ".toScala()",
                    "com.databricks.dicer.external.javaapi.SliceKey::toScala",
                    "src/main/java/com/databricks/dicer/external/javaapi/SliceKey.java",
                ),
            ),
        ] {
            let candidates = [distractor, expected.clone()]
                .into_iter()
                .map(|node| SearchCandidate {
                    node,
                    sources: BTreeSet::from([CandidateSource::Alias]),
                    indexed_matches: BTreeSet::new(),
                    relationship_matches: BTreeSet::new(),
                })
                .collect();
            let terms = crate::text::query_terms(question);

            let ranked = rank_search_candidates(question, &terms, candidates, usize::MAX);

            assert_eq!(ranked[0].node_id, expected.id, "{question}");
        }
    }
}
