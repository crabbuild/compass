//! Optional native latent-semantic analysis (LSA), learned from this graph only.
//! TF-IDF rows are projected onto deterministic truncated-SVD directions. No
//! hash vectors, remote model, credentials or runtime download are involved.
use crate::code_query::{CodeGraphBackend, CodeQueryEngine};
use crate::{QueryError, QueryErrorKind};
use compass_model::code_graph::{NodeDetails, NodeRecord};
use std::collections::{BTreeMap, BTreeSet};

const MAX_NODES: usize = 32_768;
const MAX_BYTES: usize = 64 * 1024 * 1024;
const MAX_TERMS: usize = 8_192;
const MAX_NONZERO: usize = 1_000_000;
const MAX_TEXT_CHARS: usize = 8_192;
const DIMENSIONS: usize = 16;
const ITERATIONS: usize = 12;
const MIN_SIMILARITY: f64 = 0.25;

struct BoundedCounter(usize);
impl std::io::Write for BoundedCounter {
    fn write(&mut self, buffer: &[u8]) -> std::io::Result<usize> {
        self.0 = self.0.saturating_add(buffer.len());
        if self.0 > MAX_BYTES {
            return Err(std::io::Error::other("semantic corpus exceeds 64 MiB"));
        }
        Ok(buffer.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

pub(crate) struct SemanticIndex {
    nodes: Vec<NodeRecord>,
    vocabulary: BTreeMap<String, (usize, f64)>,
    directions: Vec<Vec<f64>>,
    embeddings: Vec<Vec<f64>>,
    rows: Vec<Vec<(usize, f64)>>,
}

fn limit(detail: &str) -> QueryError {
    QueryError::new(
        QueryErrorKind::ExpansionLimit,
        "semantic_corpus_limit",
        format!(
            "Local semantic index {detail}; extract a smaller scoped graph or omit --semantic-search"
        ),
    )
}

fn corpus_text(node: &NodeRecord) -> String {
    let mut text = format!("{} {}", node.name, node.qualified_name);
    // Python's published rationale names retain bounded docstring excerpts;
    // document content retains full source-backed Markdown prose within caps.
    if let Some(NodeDetails::Document(details)) = &node.details {
        if let Some(section) = &details.section {
            text.push(' ');
            text.push_str(section);
        }
        if let Some(content) = &details.content {
            text.push(' ');
            text.extend(content.chars().take(MAX_TEXT_CHARS));
        }
    }
    if let Some(NodeDetails::Resource(details)) = &node.details
        && let Some(content) = &details.content
    {
        text.push(' ');
        text.extend(content.chars().take(MAX_TEXT_CHARS));
    }
    text
}

fn norm(vector: &mut [f64]) -> bool {
    let size = vector.iter().map(|v| v * v).sum::<f64>().sqrt();
    if size <= 1e-10 || !size.is_finite() {
        return false;
    }
    for v in vector {
        *v /= size;
    }
    true
}

fn orthogonalize(vector: &mut [f64], directions: &[Vec<f64>]) {
    for basis in directions {
        let dot = vector.iter().zip(basis).map(|(a, b)| a * b).sum::<f64>();
        for (value, basis) in vector.iter_mut().zip(basis) {
            *value -= dot * basis;
        }
    }
}

impl SemanticIndex {
    fn build(
        nodes: Vec<NodeRecord>,
        check: impl Fn() -> Result<(), QueryError>,
    ) -> Result<Self, QueryError> {
        let mut frequencies = BTreeMap::<String, usize>::new();
        let mut documents = Vec::new();
        let mut nonzero = 0;
        for node in &nodes {
            check()?;
            let tokens = crate::query_terms(&corpus_text(node))
                .into_iter()
                .take(512)
                .collect::<BTreeSet<_>>();
            nonzero += tokens.len();
            if nonzero > MAX_NONZERO {
                return Err(limit("exceeds 1,000,000 sparse entries"));
            }
            for token in &tokens {
                *frequencies.entry(token.clone()).or_default() += 1;
            }
            documents.push(tokens);
        }
        if frequencies.len() > MAX_TERMS {
            return Err(limit("exceeds 8,192 terms"));
        }
        let vocabulary = frequencies
            .into_iter()
            .enumerate()
            .map(|(index, (term, count))| {
                let idf = ((nodes.len() as f64 + 1.0) / (count as f64 + 1.0)).ln() + 1.0;
                (term, (index, idf))
            })
            .collect::<BTreeMap<_, _>>();
        check()?;
        let matrix = documents
            .into_iter()
            .map(|terms| {
                check()?;
                let mut row = terms
                    .into_iter()
                    .filter_map(|term| vocabulary.get(&term).copied())
                    .collect::<Vec<_>>();
                let size = row.iter().map(|(_, v)| v * v).sum::<f64>().sqrt();
                if size > 0.0 {
                    for (_, value) in &mut row {
                        *value /= size;
                    }
                }
                Ok(row)
            })
            .collect::<Result<Vec<_>, QueryError>>()?;
        let mut directions: Vec<Vec<f64>> = Vec::new();
        // Power iteration of A^T A with reorthogonalization. Integer seed
        // arithmetic and ordered sparse rows make equivalent inputs stable.
        for component in 0..DIMENSIONS
            .min(vocabulary.len())
            .min(nodes.len().isqrt().max(1))
        {
            check()?;
            let mut vector = (0..vocabulary.len())
                .map(|index| {
                    let seed = (index as u64 + 1)
                        .wrapping_mul(6364136223846793005)
                        .wrapping_add((component as u64).wrapping_mul(1442695040888963407));
                    ((seed >> 32) as f64 / u32::MAX as f64) - 0.5
                })
                .collect::<Vec<_>>();
            orthogonalize(&mut vector, &directions);
            if !norm(&mut vector) {
                break;
            }
            for _ in 0..ITERATIONS {
                check()?;
                let mut next = vec![0.0; vocabulary.len()];
                for row in &matrix {
                    check()?;
                    let dot = row
                        .iter()
                        .map(|(index, weight)| vector[*index] * weight)
                        .sum::<f64>();
                    for (index, weight) in row {
                        next[*index] += dot * weight;
                    }
                }
                orthogonalize(&mut next, &directions);
                if !norm(&mut next) {
                    break;
                }
                vector = next;
            }
            // Degenerate directions do not supply arbitrary similarity.
            let energy = matrix
                .iter()
                .map(|row| {
                    check()?;
                    Ok(row.iter().map(|(i, w)| vector[*i] * w).sum::<f64>().powi(2))
                })
                .collect::<Result<Vec<_>, QueryError>>()?
                .into_iter()
                .sum::<f64>();
            if energy <= 1e-8 {
                continue;
            }
            directions.push(vector);
        }
        let embeddings = matrix
            .iter()
            .map(|row| {
                check()?;
                let mut embedding = directions
                    .iter()
                    .map(|basis| row.iter().map(|(i, w)| basis[*i] * w).sum())
                    .collect::<Vec<f64>>();
                norm(&mut embedding);
                Ok(embedding)
            })
            .collect::<Result<Vec<_>, QueryError>>()?;
        check()?;
        Ok(Self {
            nodes,
            vocabulary,
            directions,
            embeddings,
            rows: matrix,
        })
    }

    fn search(
        &self,
        query: &str,
        max_nodes: u32,
        check: &dyn Fn() -> Result<(), QueryError>,
    ) -> Result<Vec<(NodeRecord, f64)>, QueryError> {
        check()?;
        let terms = crate::query_terms(query)
            .into_iter()
            .filter_map(|term| self.vocabulary.get(&term).copied())
            .collect::<BTreeMap<_, _>>();
        let query_indices = terms.keys().copied().collect::<BTreeSet<_>>();
        let mut bridges = BTreeSet::new();
        for row in &self.rows {
            check()?;
            if row.iter().any(|(index, _)| query_indices.contains(index)) {
                bridges.extend(row.iter().map(|(index, _)| *index));
            }
        }
        let mut vector = self
            .directions
            .iter()
            .map(|basis| terms.iter().map(|(i, w)| basis[*i] * w).sum())
            .collect::<Vec<f64>>();
        if !norm(&mut vector) {
            check()?;
            return Ok(Vec::new());
        }
        let mut scores = Vec::new();
        for (index, embedding) in self.embeddings.iter().enumerate() {
            check()?;
            if !self.rows[index]
                .iter()
                .any(|(index, _)| bridges.contains(index))
            {
                continue;
            }
            let similarity = vector
                .iter()
                .zip(embedding)
                .map(|(a, b)| a * b)
                .sum::<f64>();
            let similarity = (similarity.clamp(-1.0, 1.0) * 1_000_000.0).round() / 1_000_000.0;
            if similarity >= MIN_SIMILARITY {
                scores.push((index, similarity));
            }
        }
        check()?;
        scores.sort_by(|(a, ascore), (b, bscore)| {
            bscore
                .total_cmp(ascore)
                .then_with(|| self.nodes[*a].id.cmp(&self.nodes[*b].id))
        });
        let result = scores
            .into_iter()
            .take(usize::try_from(max_nodes).unwrap_or(usize::MAX).min(8))
            .map(|(index, score)| (self.nodes[index].clone(), score))
            .collect();
        check()?;
        Ok(result)
    }
}

impl CodeQueryEngine {
    pub(crate) fn semantic_candidates(
        &self,
        query: &str,
        max_nodes: u32,
    ) -> Result<Vec<(NodeRecord, f64)>, QueryError> {
        self.semantic_candidates_guarded(query, max_nodes, &|| self.check_deadline())
    }

    pub(crate) fn semantic_candidates_guarded(
        &self,
        query: &str,
        max_nodes: u32,
        check: &dyn Fn() -> Result<(), QueryError>,
    ) -> Result<Vec<(NodeRecord, f64)>, QueryError> {
        check()?;
        // A query waiting behind another index build retains its own absolute
        // deadline/cancellation guard instead of blocking in Mutex::lock.
        let mut cache = loop {
            check()?;
            match self.semantic_index.try_lock() {
                Ok(cache) => break cache,
                Err(std::sync::TryLockError::Poisoned(error)) => break error.into_inner(),
                Err(std::sync::TryLockError::WouldBlock) => std::thread::yield_now(),
            }
        };
        if cache.is_none() {
            let mut nodes = match &self.backend {
                CodeGraphBackend::Materialized { graph, .. } => {
                    if graph.nodes.len() > MAX_NODES {
                        return Err(limit("exceeds 32,768 nodes"));
                    }
                    serde_json::to_writer(BoundedCounter(0), &graph.nodes)
                        .map_err(|error| limit(&error.to_string()))?;
                    graph.nodes.clone()
                }
                CodeGraphBackend::Store(snapshot) => snapshot
                    .reader()?
                    .nodes(compass_graph::SnapshotReadLimits {
                        max_items: MAX_NODES,
                        max_objects: MAX_NODES,
                        max_bytes: MAX_BYTES,
                        ..Default::default()
                    })
                    .map_err(|error| limit(&format!("cannot read bounded corpus: {error}")))?,
            };
            check()?;
            nodes.sort_by(|a, b| a.id.cmp(&b.id));
            check()?;
            // Publish the cache only after successful, bounded construction.
            let index = SemanticIndex::build(nodes, check)?;
            check()?;
            *cache = Some(index);
        }
        match cache.as_ref() {
            Some(index) => index.search(query, max_nodes, check),
            None => Ok(Vec::new()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use compass_model::code_graph::NodeKind;

    fn node(id: &str, text: &str) -> NodeRecord {
        NodeRecord {
            id: id.into(),
            kind: NodeKind::Class,
            roles: Vec::new(),
            name: text.into(),
            qualified_name: text.into(),
            language: None,
            framework: None,
            source: None,
            details: None,
            evidence: Vec::new(),
            coverage: Vec::new(),
            diagnostics: Vec::new(),
            community: None,
        }
    }

    #[test]
    fn latent_embeddings_bridge_corpus_terms_but_not_disconnected_or_unknown_concepts()
    -> Result<(), QueryError> {
        let corpus = vec![
            node("orchard", "orchard apples harvest"),
            node("fruit", "fruit apples harvest"),
            node("billing", "invoice payment refund"),
        ];
        let index = SemanticIndex::build(corpus.clone(), || Ok(()))?;
        let result = index.search("orchard", 3, &|| Ok(()))?;
        assert!(
            result.iter().any(|(node, _)| node.id == "fruit"),
            "{result:?}"
        );
        assert!(result.iter().all(|(node, _)| node.id != "billing"));
        assert!(index.search("absentvocabulary", 3, &|| Ok(()))?.is_empty());
        let repeated = SemanticIndex::build(corpus, || Ok(()))?;
        assert_eq!(index.directions, repeated.directions);
        assert_eq!(index.embeddings, repeated.embeddings);
        Ok(())
    }

    #[test]
    fn excessive_vocabulary_is_an_explicit_limit_error() {
        let corpus = (0..20)
            .map(|row| {
                node(
                    &format!("node{row}"),
                    &(0..500)
                        .map(|term| format!("v{}x", row * 500 + term))
                        .collect::<Vec<_>>()
                        .join(" "),
                )
            })
            .collect();
        let result = SemanticIndex::build(corpus, || Ok(()));
        assert!(
            matches!(result, Err(error) if error.kind() == QueryErrorKind::ExpansionLimit && error.code() == "semantic_corpus_limit")
        );
    }

    #[test]
    fn semantic_build_deadline_failure_does_not_return_an_empty_index() {
        let error = SemanticIndex::build(vec![node("fruit", "fruit apples")], || {
            Err(QueryError::new(
                QueryErrorKind::Timeout,
                "timeout",
                "expired",
            ))
        });
        assert!(matches!(error, Err(error) if error.kind() == QueryErrorKind::Timeout));
    }
    #[test]
    fn cached_semantic_search_observes_guard_while_scanning_rows() -> Result<(), QueryError> {
        let index = SemanticIndex::build(
            (0..64)
                .map(|at| node(&format!("n{at}"), "fruit apples harvest"))
                .collect(),
            || Ok(()),
        )?;
        let checks = std::cell::Cell::new(0);
        let error = index.search("fruit", 5, &|| {
            checks.set(checks.get() + 1);
            if checks.get() >= 12 {
                Err(QueryError::new(
                    QueryErrorKind::Cancelled,
                    "test_cancelled",
                    "cancelled during cached semantic search",
                ))
            } else {
                Ok(())
            }
        });
        assert!(matches!(error, Err(error) if error.kind() == QueryErrorKind::Cancelled));
        assert_eq!(checks.get(), 12);
        Ok(())
    }

    #[test]
    fn semantic_build_checks_guard_after_tokenization_before_publishing() {
        let checks = std::cell::Cell::new(0);
        let result = SemanticIndex::build(
            (0..32)
                .map(|at| node(&format!("n{at}"), "fruit apples harvest"))
                .collect(),
            || {
                checks.set(checks.get() + 1);
                if checks.get() >= 48 {
                    Err(QueryError::new(
                        QueryErrorKind::Timeout,
                        "test_timeout",
                        "expired in matrix construction",
                    ))
                } else {
                    Ok(())
                }
            },
        );
        assert!(matches!(result, Err(error) if error.kind() == QueryErrorKind::Timeout));
        assert_eq!(checks.get(), 48);
    }
}
