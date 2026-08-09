use std::cmp::Ordering;
use std::collections::{HashMap, HashSet};
use std::path::Path;

use compass_model::{Graph, NodeIndex, NodeRecord};

use crate::text::{canonical_query_token, search_tokens, strip_diacritics};

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

struct NodeText {
    node: NodeIndex,
    normalized_label: String,
    bare_label: String,
    label_tokens: String,
}

#[must_use]
pub fn score_nodes(graph: &Graph, terms: &[String], collect_per_term_seeds: bool) -> QueryScores {
    if let [exact_id] = terms
        && exact_id.starts_with("sha256:")
        && let Some(node) = graph.node_index(exact_id)
    {
        return QueryScores {
            ranked: vec![ScoredNode {
                score: EXACT_MATCH_BONUS * 10.0,
                node,
            }],
            best_seed_by_term: if collect_per_term_seeds {
                HashMap::from([(exact_id.clone(), node)])
            } else {
                HashMap::new()
            },
        };
    }
    let mut normalized_terms = Vec::new();
    let mut seen = HashSet::new();
    for term in terms {
        for token in search_tokens(term) {
            let token = canonical_query_token(token);
            if seen.insert(token.clone()) {
                normalized_terms.push(token);
            }
        }
    }
    let term_count = normalized_terms.len();
    if term_count == 0 {
        return QueryScores::default();
    }
    let node_text = graph
        .nodes()
        .map(|(node, record)| {
            let label = record.string("label");
            let normalized_label = normalized_label_with_text(record, &label);
            let bare_label = normalized_label.trim_end_matches(['(', ')']).to_owned();
            let label_tokens = canonical_label_tokens(&label);
            NodeText {
                node,
                normalized_label,
                bare_label,
                label_tokens,
            }
        })
        .collect::<Vec<_>>();
    let idf = compute_idf(&node_text, &normalized_terms);
    let joined = normalized_terms.join(" ");
    let joined_weight = normalized_terms
        .iter()
        .filter_map(|term| idf.get(term))
        .copied()
        .fold(1.0_f64, f64::max);

    let mut ranked = Vec::new();
    let mut best: HashMap<String, BestSeed> = HashMap::new();
    let mut tie_breakers = HashMap::new();
    for text in &node_text {
        let node_index = text.node;
        let node = graph.node(node_index);
        let norm_label = text.normalized_label.as_str();
        let bare_label = text.bare_label.as_str();
        let label_tokens = text.label_tokens.as_str();
        let source = node.string("source_file").to_lowercase();
        let node_id = node.id.to_lowercase();
        let mut tie_breaker = None;
        let mut score = 0.0;
        let compound_term_matches = normalized_terms
            .iter()
            .filter(|term| term.len() >= 3 && label_has_exact_term(&text.label_tokens, term))
            .count();
        score += query_match_tier(
            norm_label,
            bare_label,
            label_tokens,
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
            let compound_token_match = compound_term_matches >= 2
                && term.len() >= 3
                && (semantic_seed_rank(node) >= 3 || node.kind_name() == "symbol")
                && label_has_exact_term(&text.label_tokens, term);
            if term == norm_label || term == bare_label || compound_token_match {
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
                let joined_tier =
                    query_match_tier(norm_label, bare_label, label_tokens, &node_id, term, weight);
                let singleton =
                    singleton_score(joined_tier, tier_value, substring_value, source_value);
                if singleton > 0.0 {
                    let tie = *tie_breaker
                        .get_or_insert_with(|| SeedTieBreaker::new(graph, node_index, node));
                    let candidate = BestSeed {
                        score: singleton,
                        tie_breaker: tie,
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
            let tie =
                *tie_breaker.get_or_insert_with(|| SeedTieBreaker::new(graph, node_index, node));
            tie_breakers.insert(node_index, tie);
            ranked.push(ScoredNode {
                score,
                node: node_index,
            });
        }
    }
    ranked.sort_by(|left, right| {
        let left_tie = tie_breakers[&left.node];
        let right_tie = tie_breakers[&right.node];
        right
            .score
            .total_cmp(&left.score)
            .then_with(|| right_tie.source_backed.cmp(&left_tie.source_backed))
            .then_with(|| right_tie.semantic_rank.cmp(&left_tie.semantic_rank))
            .then_with(|| left_tie.test_or_generated.cmp(&right_tie.test_or_generated))
            .then_with(|| {
                right_tie
                    .source_backed_degree
                    .cmp(&left_tie.source_backed_degree)
            })
            .then_with(|| left_tie.label_len.cmp(&right_tie.label_len))
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
        let key = seed_label_key(node);
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
        let key = seed_label_key(record);
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
    if let Some(index) = graph.node_index(label.trim()) {
        return vec![index];
    }
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

pub(crate) fn find_exact_nodes(graph: &Graph, label: &str) -> Vec<NodeIndex> {
    if let Some(index) = graph.node_index(label.trim()) {
        return vec![index];
    }
    let term = search_tokens(label).join(" ");
    if term.is_empty() {
        return Vec::new();
    }
    let norm_query = strip_diacritics(label).to_lowercase().trim().to_owned();
    graph
        .nodes()
        .filter_map(|(index, node)| {
            let norm_label = normalized_label(node);
            let bare_label = norm_label.trim_end_matches(['(', ')']);
            let label_tokens = search_tokens(&node.string("label")).join(" ");
            let node_id = node.id.to_lowercase();
            (term == norm_label
                || term == bare_label
                || term == label_tokens
                || term == node_id
                || norm_query == norm_label
                || norm_query == bare_label)
                .then_some(index)
        })
        .collect()
}

fn compute_idf(nodes: &[NodeText], terms: &[String]) -> HashMap<String, f64> {
    let mut frequencies = terms
        .iter()
        .map(|term| (term.clone(), 0_usize))
        .collect::<HashMap<_, _>>();
    for node in nodes {
        let label = &node.normalized_label;
        for term in terms {
            if (label.contains(term) || label_has_exact_term(&node.label_tokens, term))
                && let Some(frequency) = frequencies.get_mut(term)
            {
                *frequency += 1;
            }
        }
    }
    let node_count = nodes.len().max(1) as f64;
    frequencies
        .into_iter()
        .map(|(term, frequency)| {
            let value = (1.0 + node_count / (1.0 + frequency as f64)).ln();
            (term, value)
        })
        .collect()
}

pub(crate) fn normalized_label(node: &NodeRecord) -> String {
    let label = node.string("label");
    normalized_label_with_text(node, &label)
}

fn normalized_label_with_text(node: &NodeRecord, label: &str) -> String {
    let stored = node.string("norm_label");
    let normalized = if stored.is_empty() {
        strip_diacritics(label).to_lowercase()
    } else {
        stored.to_lowercase()
    };
    normalized.trim_start_matches('.').to_owned()
}

fn canonical_label_tokens(label: &str) -> String {
    let mut terms = String::new();
    for token in search_tokens(label) {
        append_label_token(&mut terms, canonical_query_token(token.clone()));
        for component in token.split('_').filter(|component| !component.is_empty()) {
            append_label_token(&mut terms, canonical_query_token(component.to_owned()));
        }
    }
    terms
}

fn append_label_token(tokens: &mut String, token: String) {
    if !tokens.is_empty() {
        tokens.push(' ');
    }
    tokens.push_str(&token);
}

fn label_has_exact_term(label_tokens: &str, term: &str) -> bool {
    label_tokens
        .split_whitespace()
        .any(|label_term| label_term == term)
}

fn semantic_seed_rank(node: &NodeRecord) -> u8 {
    match node.kind_name() {
        "method" | "function" | "constructor" => 4,
        "class" | "interface" | "struct" | "trait" | "type_alias" => 3,
        "module" | "package" | "namespace" | "file" => 2,
        "field" | "parameter" | "variable" | "constant" => 1,
        _ => 0,
    }
}

fn source_is_test_or_generated(node: &NodeRecord) -> bool {
    let source = node.string("source_file").to_lowercase();
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

fn seed_label_key(node: &NodeRecord) -> String {
    let key = normalized_label(node)
        .trim_end_matches(['(', ')'])
        .to_owned();
    if key.is_empty() { node.id.clone() } else { key }
}

fn source_backed_degree(graph: &Graph, node: NodeIndex) -> usize {
    graph
        .outgoing_edges(node)
        .chain(graph.incoming_edges(node))
        .filter_map(|edge| graph.edge_endpoints(edge))
        .filter(|(source, target)| {
            let neighbor = if *source == node { *target } else { *source };
            graph
                .node(neighbor)
                .source_file()
                .is_some_and(|source| !source.is_empty())
        })
        .count()
}

#[derive(Clone, Copy)]
struct SeedTieBreaker {
    source_backed: bool,
    semantic_rank: u8,
    test_or_generated: bool,
    source_backed_degree: usize,
    label_len: usize,
}

impl SeedTieBreaker {
    fn new(graph: &Graph, node_index: NodeIndex, node: &NodeRecord) -> Self {
        Self {
            source_backed: node.source_file().is_some_and(|source| !source.is_empty()),
            semantic_rank: semantic_seed_rank(node),
            test_or_generated: source_is_test_or_generated(node),
            source_backed_degree: source_backed_degree(graph, node_index),
            label_len: node.label().chars().count(),
        }
    }

    fn cmp_preference(&self, other: &Self) -> Ordering {
        self.source_backed
            .cmp(&other.source_backed)
            .then_with(|| self.semantic_rank.cmp(&other.semantic_rank))
            .then_with(|| other.test_or_generated.cmp(&self.test_or_generated))
            .then_with(|| self.source_backed_degree.cmp(&other.source_backed_degree))
            .then_with(|| other.label_len.cmp(&self.label_len))
    }
}

struct BestSeed {
    score: f64,
    tie_breaker: SeedTieBreaker,
    id: String,
    node: NodeIndex,
}

impl BestSeed {
    fn better_than(&self, other: &Self) -> bool {
        match self.score.total_cmp(&other.score) {
            Ordering::Greater => true,
            Ordering::Less => false,
            Ordering::Equal => match self.tie_breaker.cmp_preference(&other.tie_breaker) {
                Ordering::Greater => true,
                Ordering::Less => false,
                Ordering::Equal => self.id < other.id,
            },
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
            tie_breaker: super::SeedTieBreaker {
                source_backed: true,
                semantic_rank: 4,
                test_or_generated: false,
                source_backed_degree: degree,
                label_len,
            },
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
    fn source_backed_seed_beats_a_higher_degree_placeholder() -> Result<(), Box<dyn Error>> {
        let document: GraphDocument = serde_json::from_value(json!({
            "directed": true,
            "multigraph": true,
            "nodes": [
                {"id":"declaration","kind":"method","name":".run()","source_file":"src/lib.rs"},
                {"id":"placeholder","kind":"function","name":"run"},
                {"id":"caller-a","kind":"method","name":"caller_a()","source_file":"src/a.rs"},
                {"id":"caller-b","kind":"method","name":"caller_b()","source_file":"src/b.rs"},
                {"id":"caller-c","kind":"method","name":"caller_c()","source_file":"src/c.rs"}
            ],
            "links": [
                {"source":"caller-a","target":"placeholder","kind":"calls"},
                {"source":"caller-b","target":"placeholder","kind":"calls"},
                {"source":"caller-c","target":"placeholder","kind":"calls"}
            ]
        }))?;
        let graph = Graph::from_document(document)?;
        let scores = score_nodes(&graph, &["run".to_owned()], true);
        let seeds = pick_seeds(&graph, &scores, 3, 0.2);

        assert_eq!(
            seeds,
            [graph.node_index("declaration").ok_or("declaration")?]
        );
        Ok(())
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
    fn canonical_v1_ids_resolve_without_tokenizing_the_digest() -> Result<(), Box<dyn Error>> {
        let id = format!("sha256:{}", "a".repeat(64));
        let document: GraphDocument = serde_json::from_value(json!({
            "nodes": [
                {"id": id, "name": "Canonical target", "kind": "function"},
                {"id": "legacy", "label": "sha256"}
            ],
            "links": []
        }))?;
        let graph = Graph::from_document(document)?;
        let scores = score_nodes(&graph, std::slice::from_ref(&id), true);

        assert_eq!(scores.ranked.len(), 1);
        assert_eq!(graph.node(scores.ranked[0].node).id, id);
        assert_eq!(
            scores.best_seed_by_term.get(&id),
            Some(&scores.ranked[0].node)
        );
        Ok(())
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

    #[test]
    fn architecture_phrase_prefers_camel_case_resolver_over_generic_url_symbol()
    -> Result<(), Box<dyn Error>> {
        let document: GraphDocument = serde_json::from_value(json!({
            "nodes": [
                {"id":"url","label":"url()","source_file":"template/defaulttags.py"},
                {"id":"resolver","label":"URLResolver","source_file":"urls/resolvers.py"}
            ],
            "links": []
        }))?;
        let graph = Graph::from_document(document)?;
        let terms = crate::text::query_terms("where is URL resolution implemented");
        let scores = score_nodes(&graph, &terms, true);

        let first = scores.ranked.first().ok_or("missing score")?;
        assert_eq!(graph.node(first.node).id, "resolver");
        Ok(())
    }

    #[test]
    fn semantic_query_prefers_callable_production_symbols_over_test_names()
    -> Result<(), Box<dyn Error>> {
        let document: GraphDocument = serde_json::from_value(json!({
            "nodes": [
                {"id":"save-variable","label":"save","kind":"variable","source_file":"tests/uploads/test_save.py"},
                {"id":"save-method","label":".save()","kind":"method","source_file":"django/db/models/base.py"},
                {"id":"model-test","label":"Model","kind":"class","source_file":"tests/gis/models.py"},
                {"id":"model-production","label":"Model","kind":"class","source_file":"django/db/models/base.py"}
            ],
            "links": [
                {"source":"model-production","target":"save-method","kind":"contains"}
            ]
        }))?;
        let graph = Graph::from_document(document)?;
        let terms = crate::text::query_terms("how does a model save data");
        let scores = score_nodes(&graph, &terms, true);
        let seeds = pick_seeds(&graph, &scores, 3, 0.2)
            .into_iter()
            .map(|index| graph.node(index).id.as_str())
            .collect::<Vec<_>>();

        assert_eq!(seeds[..2], ["save-method", "model-production"]);
        Ok(())
    }
}
