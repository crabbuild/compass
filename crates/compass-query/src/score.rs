use std::cmp::Ordering;
use std::collections::{HashMap, HashSet};
use std::path::Path;

use compass_model::{Graph, NodeIndex, NodeRecord};

use crate::text::{search_tokens, strip_diacritics};

const EXACT_MATCH_BONUS: f64 = 1000.0;
const PREFIX_MATCH_BONUS: f64 = 100.0;
const SUBSTRING_MATCH_BONUS: f64 = 1.0;
const SOURCE_MATCH_BONUS: f64 = 0.5;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ScoredNode {
    pub score: f64,
    pub node: NodeIndex,
}

#[derive(Clone, Debug, Default)]
pub struct QueryScores {
    pub ranked: Vec<ScoredNode>,
    pub best_seed_by_term: HashMap<String, NodeIndex>,
}

#[must_use]
pub fn score_nodes(graph: &Graph, terms: &[String], collect_per_term_seeds: bool) -> QueryScores {
    let mut normalized_terms = Vec::new();
    let mut seen = HashSet::new();
    for term in terms {
        for token in search_tokens(term) {
            if seen.insert(token.clone()) {
                normalized_terms.push(token);
            }
        }
    }
    let term_count = normalized_terms.len();
    if term_count == 0 {
        return QueryScores::default();
    }
    let idf = compute_idf(graph, &normalized_terms);
    let joined = normalized_terms.join(" ");
    let joined_weight = normalized_terms
        .iter()
        .filter_map(|term| idf.get(term))
        .copied()
        .fold(1.0_f64, f64::max);

    let mut ranked = Vec::new();
    let mut best: HashMap<String, BestSeed> = HashMap::new();
    for (node_index, node) in graph.nodes() {
        let norm_label = normalized_label(node);
        let bare_label = norm_label.trim_end_matches(['(', ')']);
        let label_tokens = search_tokens(&node.string("label")).join(" ");
        let source = node.string("source_file").to_lowercase();
        let node_id = node.id.to_lowercase();
        let mut score = 0.0;
        score += query_match_tier(
            &norm_label,
            bare_label,
            &label_tokens,
            &node_id,
            &joined,
            joined_weight,
        );

        let mut matched = 0_usize;
        let mut tiered = 0.0;
        for term in &normalized_terms {
            let weight = idf.get(term).copied().unwrap_or(1.0);
            let mut tier_value = 0.0;
            let mut substring_value = 0.0;
            let mut source_value = 0.0;
            if term == &norm_label || term == bare_label {
                tier_value = EXACT_MATCH_BONUS * weight;
                matched += 1;
            } else if norm_label.starts_with(term) {
                tier_value = PREFIX_MATCH_BONUS * weight;
                matched += 1;
            } else if norm_label.contains(term) {
                substring_value = SUBSTRING_MATCH_BONUS * weight;
                score += substring_value;
                matched += 1;
            }
            if source.contains(term) {
                source_value = SOURCE_MATCH_BONUS * weight;
                score += source_value;
            }
            tiered += tier_value;

            if collect_per_term_seeds {
                let joined_tier = query_match_tier(
                    &norm_label,
                    bare_label,
                    &label_tokens,
                    &node_id,
                    term,
                    weight,
                );
                let singleton =
                    singleton_score(joined_tier, tier_value, substring_value, source_value);
                if singleton > 0.0 {
                    let candidate = BestSeed {
                        score: singleton,
                        degree: graph.degree(node_index),
                        label_len: node.label().chars().count(),
                        id: node.id.clone(),
                        node: node_index,
                    };
                    if best
                        .get(term)
                        .is_none_or(|current| candidate.better_than(current))
                    {
                        best.insert(term.clone(), candidate);
                    }
                }
            }
        }
        let coverage = matched as f64 / term_count as f64;
        score += tiered * coverage.powi(2);
        if score > 0.0 {
            ranked.push(ScoredNode {
                score,
                node: node_index,
            });
        }
    }
    ranked.sort_by(|left, right| {
        right
            .score
            .total_cmp(&left.score)
            .then_with(|| {
                graph
                    .node(left.node)
                    .label()
                    .chars()
                    .count()
                    .cmp(&graph.node(right.node).label().chars().count())
            })
            .then_with(|| graph.node(left.node).id.cmp(&graph.node(right.node).id))
    });
    QueryScores {
        ranked,
        best_seed_by_term: best
            .into_iter()
            .map(|(term, seed)| (term, seed.node))
            .collect(),
    }
}

fn query_match_tier(
    norm_label: &str,
    bare_label: &str,
    label_tokens: &str,
    node_id: &str,
    term: &str,
    weight: f64,
) -> f64 {
    if [norm_label, bare_label, label_tokens, node_id].contains(&term) {
        EXACT_MATCH_BONUS * 10.0 * weight
    } else if norm_label.starts_with(term) || label_tokens.starts_with(term) {
        PREFIX_MATCH_BONUS * 10.0 * weight
    } else {
        0.0
    }
}

fn singleton_score(joined: f64, tiered: f64, substring: f64, source: f64) -> f64 {
    joined + tiered + substring + source
}

#[must_use]
pub fn pick_scored_endpoint(graph: &Graph, scored: &[ScoredNode], query: &str) -> NodeIndex {
    let query_tokens = search_tokens(query).into_iter().collect::<HashSet<_>>();
    if query_tokens.is_empty() {
        return scored[0].node;
    }
    scored
        .iter()
        .find(|candidate| {
            let label_tokens = search_tokens(graph.node(candidate.node).label())
                .into_iter()
                .collect::<HashSet<_>>();
            query_tokens.is_subset(&label_tokens)
        })
        .map_or(scored[0].node, |candidate| candidate.node)
}

#[must_use]
pub fn pick_seeds(
    graph: &Graph,
    scores: &QueryScores,
    max_count: usize,
    gap_ratio: f64,
) -> Vec<NodeIndex> {
    let Some(first) = scores.ranked.first() else {
        return Vec::new();
    };
    let mut seeds = Vec::new();
    let mut labels = HashSet::new();
    for candidate in &scores.ranked {
        if seeds.len() >= max_count {
            break;
        }
        if !seeds.is_empty() && candidate.score < first.score * gap_ratio {
            break;
        }
        let node = graph.node(candidate.node);
        let key = normalized_label(node);
        let key = if key.is_empty() { node.id.clone() } else { key };
        if labels.insert(key) {
            seeds.push(candidate.node);
        }
    }
    let mut terms = scores.best_seed_by_term.keys().collect::<Vec<_>>();
    terms.sort();
    for term in terms {
        let node = scores.best_seed_by_term[term];
        let record = graph.node(node);
        let key = normalized_label(record);
        let key = if key.is_empty() {
            record.id.clone()
        } else {
            key
        };
        if !seeds.contains(&node) && labels.insert(key) {
            seeds.push(node);
        }
    }
    seeds
}

#[must_use]
pub fn find_node(graph: &Graph, label: &str) -> Vec<NodeIndex> {
    let term = search_tokens(label).join(" ");
    if term.is_empty() {
        return Vec::new();
    }
    let norm_query = strip_diacritics(label).to_lowercase().trim().to_owned();
    let mut source_exact = Vec::new();
    let mut exact = Vec::new();
    let mut prefix = Vec::new();
    let mut substring = Vec::new();
    for (index, node) in graph.nodes() {
        let norm_label = normalized_label(node);
        let bare_label = norm_label.trim_end_matches(['(', ')']);
        let label_tokens = search_tokens(&node.string("label")).join(" ");
        let source_tokens = search_tokens(&node.string("source_file")).join(" ");
        let node_id = node.id.to_lowercase();
        if term == source_tokens {
            source_exact.push(index);
        } else if term == norm_label
            || term == bare_label
            || term == label_tokens
            || term == node_id
            || norm_query == norm_label
            || norm_query == bare_label
        {
            exact.push(index);
        } else if norm_label.starts_with(&term)
            || bare_label.starts_with(&term)
            || label_tokens.starts_with(&term)
            || node_id.starts_with(&term)
            || norm_label.starts_with(&norm_query)
            || bare_label.starts_with(&norm_query)
        {
            prefix.push(index);
        } else if norm_label.contains(&term)
            || label_tokens.contains(&term)
            || norm_label.contains(&norm_query)
        {
            substring.push(index);
        }
    }
    if !source_exact.is_empty() {
        let basename = Path::new(label)
            .file_name()
            .and_then(|name| name.to_str())
            .map_or_else(String::new, |name| strip_diacritics(name).to_lowercase());
        let preferred = source_exact
            .iter()
            .copied()
            .filter(|&index| {
                let node = graph.node(index);
                node.string("source_location") == "L1"
                    && strip_diacritics(&node.string("label")).to_lowercase() == basename
            })
            .collect::<Vec<_>>();
        if preferred.len() == 1 {
            let winner = preferred[0];
            source_exact.retain(|index| *index != winner);
            source_exact.insert(0, winner);
        }
    }
    source_exact.extend(exact);
    source_exact.extend(prefix);
    source_exact.extend(substring);
    source_exact
}

fn compute_idf(graph: &Graph, terms: &[String]) -> HashMap<String, f64> {
    let mut frequencies = terms
        .iter()
        .map(|term| (term.clone(), 0_usize))
        .collect::<HashMap<_, _>>();
    for (_, node) in graph.nodes() {
        let label = normalized_label(node);
        for term in terms {
            if label.contains(term)
                && let Some(frequency) = frequencies.get_mut(term)
            {
                *frequency += 1;
            }
        }
    }
    let node_count = graph.node_count().max(1) as f64;
    frequencies
        .into_iter()
        .map(|(term, frequency)| {
            let value = (1.0 + node_count / (1.0 + frequency as f64)).ln();
            (term, value)
        })
        .collect()
}

pub(crate) fn normalized_label(node: &NodeRecord) -> String {
    let stored = node.string("norm_label");
    if stored.is_empty() {
        strip_diacritics(&node.string("label")).to_lowercase()
    } else {
        stored.to_lowercase()
    }
}

struct BestSeed {
    score: f64,
    degree: usize,
    label_len: usize,
    id: String,
    node: NodeIndex,
}

impl BestSeed {
    fn better_than(&self, other: &Self) -> bool {
        match self.score.total_cmp(&other.score) {
            Ordering::Greater => true,
            Ordering::Less => false,
            Ordering::Equal => {
                self.degree > other.degree
                    || (self.degree == other.degree
                        && (self.label_len < other.label_len
                            || (self.label_len == other.label_len && self.id < other.id)))
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;
    use std::error::Error;

    use compass_model::{Graph, GraphDocument};
    use serde_json::json;

    use super::{
        BestSeed, QueryScores, ScoredNode, pick_seeds, query_match_tier, score_nodes,
        singleton_score,
    };

    fn seed(score: f64, degree: usize, label_len: usize, id: &str) -> BestSeed {
        BestSeed {
            score,
            degree,
            label_len,
            id: id.to_owned(),
            node: 0,
        }
    }

    #[test]
    fn best_seed_order_checks_every_tie_breaker() {
        let baseline = seed(10.0, 2, 5, "b");
        assert!(seed(11.0, 0, 99, "z").better_than(&baseline));
        assert!(!seed(9.0, 99, 1, "a").better_than(&baseline));
        assert!(seed(10.0, 3, 99, "z").better_than(&baseline));
        assert!(!seed(10.0, 1, 1, "a").better_than(&baseline));
        assert!(seed(10.0, 2, 4, "z").better_than(&baseline));
        assert!(!seed(10.0, 2, 6, "a").better_than(&baseline));
        assert!(seed(10.0, 2, 5, "a").better_than(&baseline));
        assert!(!seed(10.0, 2, 5, "c").better_than(&baseline));
        assert!(!seed(10.0, 2, 5, "b").better_than(&baseline));
    }

    #[test]
    fn query_tier_and_singleton_arithmetic_are_exact() {
        assert_eq!(
            query_match_tier("alpha", "alpha", "alpha", "id", "alpha", 2.0),
            20_000.0
        );
        assert_eq!(
            query_match_tier("alpha", "alpha", "words", "id", "al", 2.0),
            2_000.0
        );
        assert_eq!(
            query_match_tier("other", "other", "alpha words", "id", "alpha", 2.0),
            2_000.0
        );
        assert_eq!(
            query_match_tier("other", "other", "words", "id", "absent", 2.0),
            0.0
        );
        assert_eq!(singleton_score(20_000.0, 2_000.0, 3.0, 0.5), 22_003.5);
    }

    #[test]
    fn seed_selection_enforces_count_gap_and_label_uniqueness() -> Result<(), Box<dyn Error>> {
        let document: GraphDocument = serde_json::from_value(json!({
            "directed": true,
            "multigraph": false,
            "graph": {},
            "nodes": [
                {"id":"a1","label":"Alpha"},
                {"id":"a2","label":"Alpha"},
                {"id":"b","label":"Beta"},
                {"id":"c","label":"Gamma"}
            ],
            "links": []
        }))?;
        let graph = Graph::from_document(document)?;
        let ranked = vec![
            ScoredNode {
                score: 10.0,
                node: 0,
            },
            ScoredNode {
                score: 9.0,
                node: 1,
            },
            ScoredNode {
                score: 1.0,
                node: 2,
            },
        ];
        let scores = QueryScores {
            ranked,
            best_seed_by_term: HashMap::from([
                ("duplicate".to_owned(), 1),
                ("unique".to_owned(), 3),
            ]),
        };

        assert_eq!(pick_seeds(&graph, &scores, 1, 0.0), [0, 3]);
        assert_eq!(pick_seeds(&graph, &scores, 3, 0.2), [0, 3]);
        let no_term_seeds = QueryScores {
            ranked: vec![
                ScoredNode {
                    score: 10.0,
                    node: 0,
                },
                ScoredNode {
                    score: 9.0,
                    node: 2,
                },
                ScoredNode {
                    score: 1.0,
                    node: 3,
                },
            ],
            best_seed_by_term: HashMap::new(),
        };
        assert_eq!(pick_seeds(&graph, &no_term_seeds, 3, 0.2), [0, 2]);
        assert_eq!(pick_seeds(&graph, &no_term_seeds, 1, 2.0), [0]);
        let boundary = QueryScores {
            ranked: vec![
                ScoredNode {
                    score: 10.0,
                    node: 0,
                },
                ScoredNode {
                    score: 2.0,
                    node: 2,
                },
                ScoredNode {
                    score: 1.0,
                    node: 3,
                },
            ],
            best_seed_by_term: HashMap::new(),
        };
        assert_eq!(pick_seeds(&graph, &boundary, 3, 0.2), [0, 2]);
        assert!(pick_seeds(&graph, &QueryScores::default(), 3, 0.2).is_empty());
        Ok(())
    }

    #[test]
    fn scoring_weights_exact_prefix_substring_source_and_bare_labels() -> Result<(), Box<dyn Error>>
    {
        let document: GraphDocument = serde_json::from_value(json!({
            "directed": true,
            "multigraph": false,
            "graph": {},
            "nodes": [
                {"id":"alias","label":"Display Name","norm_label":"alias","source_file":"src/special.rs"},
                {"id":"run","label":"run()"},
                {"id":"other","label":"OtherThing"},
                {"id":"plain-a","label":"Plain A"},
                {"id":"plain-b","label":"Plain B"}
            ],
            "links": []
        }))?;
        let graph = Graph::from_document(document)?;
        let rare = 3.5_f64.ln();
        let absent = 6.0_f64.ln();
        let cases = [
            ("alias", "alias", 11_000.0 * rare),
            ("ali", "alias", 1_100.0 * rare),
            ("run", "run", 11_000.0 * rare),
            ("thing", "other", rare),
            ("special", "alias", 0.5 * absent),
        ];
        for (term, expected_id, expected_score) in cases {
            let scores = score_nodes(&graph, &[term.to_owned()], true);
            let first = scores.ranked.first().ok_or("missing score")?;
            assert_eq!(graph.node(first.node).id, expected_id);
            assert!(
                (first.score - expected_score).abs() < 1e-12,
                "{term}: actual={}, expected={expected_score}",
                first.score
            );
        }
        let no_match = score_nodes(&graph, &["absent".to_owned()], true);
        assert!(no_match.ranked.is_empty());
        assert!(no_match.best_seed_by_term.is_empty());

        let combined = score_nodes(&graph, &["plain".to_owned(), "a".to_owned()], false);
        let first = combined.ranked.first().ok_or("missing combined score")?;
        assert_eq!(graph.node(first.node).id, "plain-a");
        let expected = 10_098.893_855_517_388;
        assert!(
            (first.score - expected).abs() < 1e-10,
            "actual={}, expected={expected}",
            first.score
        );
        Ok(())
    }

    #[test]
    fn dangling_endpoint_ids_do_not_become_query_labels() -> Result<(), Box<dyn Error>> {
        let document: GraphDocument = serde_json::from_value(json!({
            "directed": false,
            "multigraph": false,
            "graph": {},
            "nodes": [{"id":"real","label":"extract()"}],
            "links": [{"source":"real","target":"extract","relation":"imports"}]
        }))?;
        let graph = Graph::from_document(document)?;

        let scores = score_nodes(
            &graph,
            &["extract".to_owned(), "unmatched".to_owned()],
            true,
        );

        assert_eq!(scores.ranked.len(), 1);
        assert_eq!(graph.node(scores.ranked[0].node).id, "real");
        assert_eq!(
            scores
                .best_seed_by_term
                .get("extract")
                .map(|index| graph.node(*index).id.as_str()),
            Some("real")
        );
        Ok(())
    }
}
