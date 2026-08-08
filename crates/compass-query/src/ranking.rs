use std::cmp::Ordering;
use std::collections::BTreeSet;

use compass_model::code_graph::{NodeKind, NodeRecord};
use compass_model::provenance::EvidenceConfidence;

use crate::recall::{CandidateSource, SearchCandidate};
use crate::text::strip_diacritics;

const QUERY_RANKER_PROFILE_V1: &str = "query-ranker/1";
const QUERY_RANKER_PROFILE_V2: &str = "query-ranker/2";

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum SearchRankProfile {
    QueryRankerV1,
    QueryRankerV2,
}

impl SearchRankProfile {
    pub(crate) fn from_env() -> Self {
        std::env::var("COMPASS_QUERY_RANKER_PROFILE")
            .ok()
            .as_deref()
            .and_then(parse_query_ranker_profile)
            .unwrap_or(Self::QueryRankerV1)
    }
}

#[derive(Clone, Debug)]
pub(crate) struct RankedSearchResult {
    pub(crate) score: f64,
    pub(crate) node_id: String,
    pub(crate) matched_fields: Vec<String>,
    pub(crate) node: NodeRecord,
}

pub(crate) fn rank_search_candidates(
    profile: SearchRankProfile,
    query: &str,
    terms: &[String],
    candidates: Vec<SearchCandidate>,
) -> Vec<RankedSearchResult> {
    match profile {
        SearchRankProfile::QueryRankerV1 => rank_legacy(query, candidates),
        SearchRankProfile::QueryRankerV2 => rank_v2(query, terms, candidates),
    }
}

fn rank_legacy(query: &str, candidates: Vec<SearchCandidate>) -> Vec<RankedSearchResult> {
    let normalized_query = strip_diacritics(query).to_lowercase();
    let mut ranked = Vec::new();

    for candidate in candidates {
        let source_rank = candidate.best_source_rank();
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
            node_id: node.id.clone(),
            matched_fields,
            node,
        });
    }

    ranked.sort_by(|left, right| {
        right
            .score
            .total_cmp(&left.score)
            .then_with(|| left.node_id.cmp(&right.node_id))
    });
    ranked
}

fn rank_v2(
    query: &str,
    terms: &[String],
    candidates: Vec<SearchCandidate>,
) -> Vec<RankedSearchResult> {
    let normalized_query = strip_diacritics(query).to_lowercase();
    let mut ranked_terms = terms
        .iter()
        .map(|term| strip_diacritics(term).to_lowercase())
        .filter(|term| !term.is_empty())
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect::<Vec<_>>();
    let query_terms = std::mem::take(&mut ranked_terms);

    let mut ranked = Vec::new();

    for candidate in candidates {
        let source_rank = candidate.best_source_rank();
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

        let lexical_score = lexical_score_v2(
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
        );
        let evidence_score = evidence_score(&candidate.node);
        let trust_score = trust_score_v2(&candidate, matched_fields.len());
        let semantic_score = semantic_signal_score(&candidate.node);
        let ambiguity_score = ambiguity_signal_score(&candidate);
        let node = candidate.node;

        let tie = SearchCandidateTiebreak::new(source_rank, &node);
        let score = lexical_score + evidence_score + trust_score + semantic_score + ambiguity_score;

        ranked.push(RankedSearchCandidate {
            score,
            tie,
            result: RankedSearchResult {
                score,
                node_id: node.id.clone(),
                matched_fields,
                node,
            },
        });
    }

    ranked.sort_by(compare_ranked_candidates);
    ranked.into_iter().map(|entry| entry.result).collect()
}

#[derive(Debug)]
struct RankedSearchCandidate {
    score: f64,
    tie: SearchCandidateTiebreak,
    result: RankedSearchResult,
}

fn compare_ranked_candidates(
    left: &RankedSearchCandidate,
    right: &RankedSearchCandidate,
) -> Ordering {
    right
        .score
        .total_cmp(&left.score)
        .then_with(|| right.tie.source_rank.cmp(&left.tie.source_rank))
        .then_with(|| right.tie.compare(&left.tie))
        .then_with(|| left.result.node_id.cmp(&right.result.node_id))
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
        EvidenceConfidence::Exact => 10_000.0,
        EvidenceConfidence::Inferred => 5_000.0,
        EvidenceConfidence::Ambiguous => 1_500.0,
    }
}

fn trust_score_v2(candidate: &SearchCandidate, matched_fields: usize) -> f64 {
    let mut score = f64::from(candidate.best_source_rank()) * 1_200.0;
    score += matched_fields as f64 * 200.0;
    score += f64::from(candidate.sources.len() as u16) * 3_000.0;
    score
}

fn semantic_signal_score(node: &NodeRecord) -> f64 {
    f64::from(semantic_seed_rank(node)) * 650.0
}

fn ambiguity_signal_score(candidate: &SearchCandidate) -> f64 {
    let source_rank = candidate.best_source_rank();
    let mut score = f64::from(source_rank) * 1_000.0;

    if candidate.sources.contains(&CandidateSource::Fuzzy) {
        score -= 12_000.0;
    }
    if candidate
        .sources
        .contains(&CandidateSource::HeuristicFallback)
    {
        score -= 8_000.0;
    }
    if candidate.sources.is_empty() {
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
    source.split('/').any(|component| {
        matches!(
            component,
            "test" | "tests" | "testing" | "fixtures" | "vendor" | "generated"
        )
    }) || source
        .rsplit('/')
        .next()
        .is_some_and(|name| name.starts_with("test_") || name.ends_with("_test.go"))
}

fn parse_query_ranker_profile(profile: &str) -> Option<SearchRankProfile> {
    match profile {
        QUERY_RANKER_PROFILE_V1 => Some(SearchRankProfile::QueryRankerV1),
        QUERY_RANKER_PROFILE_V2 => Some(SearchRankProfile::QueryRankerV2),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use compass_model::code_graph::{NodeKind, NodeRecord};
    use compass_model::provenance::{EvidenceConfidence, EvidenceOrigin, Provenance, SourceAnchor};

    use crate::recall::{CandidateSource, SearchCandidate};

    use super::{
        SearchRankProfile::QueryRankerV1, SearchRankProfile::QueryRankerV2,
        parse_query_ranker_profile, rank_search_candidates,
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

    #[test]
    fn parse_ranker_profile_unknown_values_fall_back_to_v1() {
        assert_eq!(
            parse_query_ranker_profile("query-ranker/1"),
            Some(QueryRankerV1)
        );
        assert_eq!(
            parse_query_ranker_profile("query-ranker/2"),
            Some(QueryRankerV2)
        );
        assert_eq!(parse_query_ranker_profile("query-ranker/3"), None);
    }

    #[test]
    fn legacy_ranking_remains_deterministic_on_ties() {
        let candidates = vec![
            SearchCandidate {
                node: node("n:z", "query", NodeKind::Function, "src/lib.rs", false),
                sources: BTreeSet::from([CandidateSource::ExactName]),
            },
            SearchCandidate {
                node: node("n:a", "query", NodeKind::Function, "src/lib.rs", false),
                sources: BTreeSet::from([CandidateSource::ExactName]),
            },
        ];
        let ranked = rank_search_candidates(
            QueryRankerV1,
            "query",
            std::slice::from_ref(&"query".to_owned()),
            candidates,
        );
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
            },
        ];
        let ranked = rank_search_candidates(
            QueryRankerV2,
            "search",
            std::slice::from_ref(&"search".to_owned()),
            candidates,
        );
        assert_eq!(ranked[0].node_id, "n:source");
    }

    #[test]
    fn profile_v2_tiebreaks_stably_for_equal_scores() {
        let candidates = vec![
            SearchCandidate {
                node: node("n:aa", "same", NodeKind::Function, "src/lib.rs", false),
                sources: BTreeSet::from([CandidateSource::Alias]),
            },
            SearchCandidate {
                node: node("n:ab", "same", NodeKind::Function, "src/lib.rs", false),
                sources: BTreeSet::from([CandidateSource::Alias]),
            },
        ];
        let ranked = rank_search_candidates(
            QueryRankerV2,
            "same",
            std::slice::from_ref(&"same".to_owned()),
            candidates,
        );
        assert_eq!(ranked[0].node_id, "n:aa");
        assert_eq!(ranked[1].node_id, "n:ab");
    }
}
