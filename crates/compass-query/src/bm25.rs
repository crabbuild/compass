use std::cmp::Ordering;
use std::collections::{BTreeMap, BTreeSet};

use compass_model::{
    Graph, LexicalTermFrequency, NodeIndex, canonical_code_token, identifier_tokens,
};

pub(crate) const BM25_PROFILE_V1: &str = "text-ranker/bm25-v1";
pub(crate) const DEFAULT_BM25_CANDIDATE_LIMIT: usize = 512;
const MAX_BM25_QUERY_TERMS: usize = 32;
const K1: f64 = 1.2;
const B: f64 = 0.75;
const LABEL_WEIGHT: f64 = 4.0;
const IDENTIFIER_WEIGHT: f64 = 3.0;
const KIND_WEIGHT: f64 = 1.0;
const SOURCE_WEIGHT: f64 = 0.5;

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Bm25Candidate {
    pub(crate) node: NodeIndex,
    pub(crate) score: f64,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct Bm25Results {
    pub(crate) ranked: Vec<Bm25Candidate>,
    pub(crate) best_by_term: BTreeMap<String, NodeIndex>,
    pub(crate) truncated: bool,
}

#[derive(Clone, Copy, Debug, Default)]
struct CandidateScore {
    score: f64,
}

pub(crate) fn retrieve(graph: &Graph, terms: &[String], limit: usize) -> Bm25Results {
    let (terms, terms_truncated) = normalized_terms(terms);
    if terms.is_empty() {
        return Bm25Results {
            truncated: terms_truncated,
            ..Bm25Results::default()
        };
    }

    let index = graph.lexical_index();
    let document_count = index.document_count();
    if document_count == 0 {
        return Bm25Results {
            truncated: terms_truncated,
            ..Bm25Results::default()
        };
    }
    let average_document_length = index.average_document_length().max(1.0);
    let mut scores = BTreeMap::<NodeIndex, CandidateScore>::new();
    let mut best_by_term = BTreeMap::<String, Bm25Candidate>::new();

    for term in &terms {
        let postings = index.postings(term);
        if postings.is_empty() {
            continue;
        }
        let idf = inverse_document_frequency(document_count, postings.len());
        for posting in postings {
            let Some(document_length) = index.document_length(posting.node) else {
                continue;
            };
            let term_score = bm25_term_score(
                weighted_frequency(posting.frequency),
                f64::from(document_length),
                average_document_length,
                idf,
            );
            if term_score <= 0.0 || !term_score.is_finite() {
                continue;
            }
            scores.entry(posting.node).or_default().score += term_score;
            let candidate = Bm25Candidate {
                node: posting.node,
                score: term_score,
            };
            if best_by_term
                .get(term)
                .is_none_or(|current| compare_candidates(graph, &candidate, current).is_lt())
            {
                best_by_term.insert(term.clone(), candidate);
            }
        }
    }

    let mut ranked = scores
        .into_iter()
        .filter_map(|(node, score)| {
            score.score.is_finite().then_some(Bm25Candidate {
                node,
                score: score.score,
            })
        })
        .collect::<Vec<_>>();
    ranked.sort_by(|left, right| compare_candidates(graph, left, right));
    let candidate_truncated = ranked.len() > limit;
    ranked = retain_pinned_candidates(graph, ranked, best_by_term.values().copied(), limit);

    Bm25Results {
        ranked,
        best_by_term: best_by_term
            .into_iter()
            .map(|(term, candidate)| (term, candidate.node))
            .collect(),
        truncated: terms_truncated || candidate_truncated,
    }
}

fn normalized_terms(terms: &[String]) -> (Vec<String>, bool) {
    let mut normalized = BTreeSet::new();
    let mut truncated = false;
    for term in terms {
        for token in identifier_tokens(term) {
            let token = canonical_code_token(token);
            if !is_searchable(&token) || normalized.contains(&token) {
                continue;
            }
            if normalized.len() >= MAX_BM25_QUERY_TERMS {
                truncated = true;
                break;
            }
            normalized.insert(token);
        }
        if truncated {
            break;
        }
    }
    (normalized.into_iter().collect(), truncated)
}

fn is_searchable(term: &str) -> bool {
    !term.is_empty()
        && (!term.chars().all(|character| character.is_ascii_lowercase())
            || term.chars().count() > 2)
}

fn inverse_document_frequency(document_count: usize, document_frequency: usize) -> f64 {
    let documents = document_count as f64;
    let frequency = document_frequency as f64;
    ((documents - frequency + 0.5) / (frequency + 0.5) + 1.0).ln()
}

fn weighted_frequency(frequency: LexicalTermFrequency) -> f64 {
    f64::from(frequency.label) * LABEL_WEIGHT
        + f64::from(frequency.identifier) * IDENTIFIER_WEIGHT
        + f64::from(frequency.kind) * KIND_WEIGHT
        + f64::from(frequency.source) * SOURCE_WEIGHT
}

fn bm25_term_score(
    term_frequency: f64,
    document_length: f64,
    average_document_length: f64,
    idf: f64,
) -> f64 {
    if term_frequency <= 0.0 || average_document_length <= 0.0 {
        return 0.0;
    }
    let numerator = term_frequency * (K1 + 1.0);
    let denominator =
        term_frequency + K1 * (1.0 - B + B * (document_length / average_document_length));
    if denominator <= 0.0 {
        0.0
    } else {
        idf * (numerator / denominator)
    }
}

fn retain_pinned_candidates(
    graph: &Graph,
    ranked: Vec<Bm25Candidate>,
    pinned: impl IntoIterator<Item = Bm25Candidate>,
    limit: usize,
) -> Vec<Bm25Candidate> {
    if limit == 0 {
        return Vec::new();
    }
    let ranked_by_node = ranked
        .iter()
        .map(|candidate| (candidate.node, *candidate))
        .collect::<BTreeMap<_, _>>();
    let mut selected = pinned
        .into_iter()
        .filter_map(|candidate| ranked_by_node.get(&candidate.node).copied())
        .collect::<Vec<_>>();
    selected.sort_by(|left, right| compare_candidates(graph, left, right));
    selected.dedup_by_key(|candidate| candidate.node);
    if selected.len() > limit {
        selected.truncate(limit);
        return selected;
    }

    let mut selected_nodes = selected
        .iter()
        .map(|candidate| candidate.node)
        .collect::<BTreeSet<_>>();
    for candidate in ranked {
        if selected.len() >= limit {
            break;
        }
        if selected_nodes.insert(candidate.node) {
            selected.push(candidate);
        }
    }
    selected.sort_by(|left, right| compare_candidates(graph, left, right));
    selected
}

fn compare_candidates(graph: &Graph, left: &Bm25Candidate, right: &Bm25Candidate) -> Ordering {
    right
        .score
        .total_cmp(&left.score)
        .then_with(|| graph.node(left.node).id.cmp(&graph.node(right.node).id))
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use compass_model::Graph;

    use super::{BM25_PROFILE_V1, retrieve};

    fn graph() -> Graph {
        let document = serde_json::from_value(json!({
            "directed": true,
            "multigraph": false,
            "graph": {},
            "nodes": [{
                "id": "n:dependency-solve",
                "label": "solve_dependencies",
                "kind": "function",
                "source_file": "src/dependencies.py"
            }, {
                "id": "n:dependency-fixture",
                "label": "dependency_fixture",
                "kind": "variable",
                "source_file": "tests/generated/dependencies.py"
            }, {
                "id": "n:route-register",
                "label": "route_register",
                "kind": "function",
                "source_file": "src/routes.py"
            }, {
                "id": "n:route-helper",
                "label": "route_helper",
                "kind": "function",
                "source_file": "src/helpers.py"
            }],
            "links": []
        }))
        .unwrap_or_else(|_| std::process::abort());
        Graph::from_document(document).unwrap_or_else(|_| std::process::abort())
    }

    #[test]
    fn fielded_bm25_is_deterministic_bounded_and_term_complete() {
        let graph = graph();
        let terms = ["dependencies".to_owned(), "solved".to_owned()];
        let first = retrieve(&graph, &terms, 2);
        let second = retrieve(&graph, &terms, 2);

        assert_eq!(BM25_PROFILE_V1, "text-ranker/bm25-v1");
        assert_eq!(first, second);
        assert_eq!(first.ranked.len(), 2);
        assert_eq!(graph.node(first.ranked[0].node).id, "n:dependency-solve");
        assert_eq!(
            graph.node(first.best_by_term["dependency"]).id,
            "n:dependency-solve"
        );
        assert_eq!(
            graph.node(first.best_by_term["solve"]).id,
            "n:dependency-solve"
        );
        assert!(!first.truncated);
    }

    #[test]
    fn fielded_bm25_surfaces_candidate_truncation_and_no_answer() {
        let graph = graph();
        let bounded = retrieve(&graph, &["route".to_owned()], 1);
        assert_eq!(bounded.ranked.len(), 1);
        assert!(bounded.truncated);

        let missing = retrieve(&graph, &["quantum".to_owned()], 512);
        assert!(missing.ranked.is_empty());
        assert!(missing.best_by_term.is_empty());
        assert!(!missing.truncated);
    }
}
