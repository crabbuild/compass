use std::cmp::Ordering;
use std::collections::BTreeSet;

use compass_model::code_graph::{NodeKind, NodeRecord};
use compass_model::provenance::EvidenceConfidence;

use crate::recall::{CandidateSource, SearchCandidate};
use crate::text::{canonical_query_token, search_tokens, strip_diacritics};

pub const QUERY_RANKER_PROFILE_V2: &str = "query-ranker/2";

#[derive(Clone, Debug)]
pub(crate) struct RankedSearchResult {
    pub(crate) score: f64,
    pub(crate) node_id: String,
    pub(crate) matched_fields: Vec<String>,
    pub(crate) matched_terms: Vec<String>,
    pub(crate) node: NodeRecord,
    pub(crate) candidate_source: CandidateSource,
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
fn rank_legacy(
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

    let mut ranked = Vec::new();

    for candidate in candidates {
        let source_rank = candidate.best_source_rank();
        let candidate_source = candidate.best_source();
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
        if !relationship_terms.is_empty() {
            matched_fields.push("relationship".to_owned());
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
        let field_score = semantic_field_score(&candidate.node, &query_terms);
        let ambiguity_score = ambiguity_signal_score(&candidate);
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
            tie,
            result: RankedSearchResult {
                score,
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
    terms.iter().fold(0.0, |score, term| {
        score
            + if behavior.contains(term) {
                18_000.0
            } else {
                0.0
            }
            + if owner.contains(term) { 32_000.0 } else { 0.0 }
    })
}

fn canonical_field_tokens(value: &str) -> BTreeSet<String> {
    search_tokens(&value.replace('_', " "))
        .into_iter()
        .map(canonical_query_token)
        .collect()
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

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use compass_model::code_graph::{NodeKind, NodeRecord};
    use compass_model::provenance::{EvidenceConfidence, EvidenceOrigin, Provenance, SourceAnchor};

    use crate::recall::{CandidateSource, SearchCandidate};

    use super::{rank_legacy, rank_search_candidates};

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
    fn legacy_ranking_remains_deterministic_on_ties() {
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
        let ranked = rank_legacy("query", candidates, usize::MAX);
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
        let legacy = rank_legacy("charge", candidates.clone(), 1);
        let current = rank_search_candidates(
            "charge",
            std::slice::from_ref(&"charge".to_owned()),
            candidates,
            1,
        );

        assert_eq!(legacy[0].node_id, "n:a-generated-charge");
        assert_eq!(current[0].node_id, "n:z-payment-charge");
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
