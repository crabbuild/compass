use std::collections::{HashMap, HashSet};

use compass_model::{EdgeRecord, GraphDocument};

use crate::query_terms;

const SAMPLE_QUESTIONS: [&str; 5] = [
    "how does authentication work",
    "what is the main entry point",
    "how are errors handled",
    "what connects the data layer to the api",
    "what are the core abstractions",
];

#[derive(Clone, Debug, PartialEq)]
pub struct BenchmarkQuestion {
    pub question: String,
    pub query_tokens: usize,
    pub reduction: f64,
}

#[derive(Clone, Debug, PartialEq)]
pub struct BenchmarkResult {
    pub error: Option<String>,
    pub corpus_tokens: usize,
    pub corpus_words: usize,
    pub nodes: usize,
    pub edges: usize,
    pub avg_query_tokens: usize,
    pub reduction_ratio: f64,
    pub per_question: Vec<BenchmarkQuestion>,
}

#[must_use]
pub fn run_benchmark(
    document: &GraphDocument,
    corpus_words: Option<usize>,
    questions: Option<&[String]>,
) -> BenchmarkResult {
    let corpus_words = corpus_words.unwrap_or(document.nodes.len() * 50);
    let corpus_tokens = corpus_words * 100 / 75;
    let defaults = SAMPLE_QUESTIONS.map(str::to_owned);
    let questions = questions.unwrap_or(&defaults);
    let graph = BenchmarkGraph::new(document);
    let mut per_question = Vec::new();
    for question in questions {
        let query_tokens = graph.query_tokens(question, 3);
        if query_tokens > 0 {
            per_question.push(BenchmarkQuestion {
                question: question.clone(),
                query_tokens,
                reduction: round_one(corpus_tokens as f64 / query_tokens as f64),
            });
        }
    }
    if per_question.is_empty() {
        return BenchmarkResult {
            error: Some(
                "No matching nodes found for sample questions. Build the graph first.".to_owned(),
            ),
            corpus_tokens,
            corpus_words,
            nodes: document.nodes.len(),
            edges: document.links.len(),
            avg_query_tokens: 0,
            reduction_ratio: 0.0,
            per_question,
        };
    }
    let avg_query_tokens = per_question
        .iter()
        .map(|question| question.query_tokens)
        .sum::<usize>()
        / per_question.len();
    let reduction_ratio = if avg_query_tokens > 0 {
        round_one(corpus_tokens as f64 / avg_query_tokens as f64)
    } else {
        0.0
    };
    BenchmarkResult {
        error: None,
        corpus_tokens,
        corpus_words,
        nodes: document.nodes.len(),
        edges: document.links.len(),
        avg_query_tokens,
        reduction_ratio,
        per_question,
    }
}

#[must_use]
pub fn format_benchmark(result: &BenchmarkResult, unicode: bool) -> String {
    if let Some(error) = &result.error {
        return format!("Benchmark error: {error}");
    }
    let rule = if unicode { "─" } else { "-" }.repeat(50);
    let arrow = if unicode { "→" } else { "->" };
    let mut lines = vec![
        String::new(),
        "graphify token reduction benchmark".to_owned(),
        rule,
        format!(
            "  Corpus:          {} words {arrow} ~{} tokens (naive)",
            grouped(result.corpus_words),
            grouped(result.corpus_tokens)
        ),
        format!(
            "  Graph:           {} nodes, {} edges",
            grouped(result.nodes),
            grouped(result.edges)
        ),
        format!(
            "  Avg query cost:  ~{} tokens",
            grouped(result.avg_query_tokens)
        ),
        format!(
            "  Reduction:       {}x fewer tokens per query",
            python_float(result.reduction_ratio)
        ),
        String::new(),
        "  Per question:".to_owned(),
    ];
    lines.extend(result.per_question.iter().map(|question| {
        let question_text = question.question.chars().take(55).collect::<String>();
        format!(
            "    [{}x] {question_text}",
            python_float(question.reduction)
        )
    }));
    lines.push(String::new());
    lines.join("\n")
}

struct BenchmarkGraph<'a> {
    document: &'a GraphDocument,
    positions: HashMap<&'a str, usize>,
    neighbors: Vec<Vec<usize>>,
    edges: HashMap<(usize, usize), &'a EdgeRecord>,
}

impl<'a> BenchmarkGraph<'a> {
    fn new(document: &'a GraphDocument) -> Self {
        let positions = document
            .nodes
            .iter()
            .enumerate()
            .map(|(index, node)| (node.id.as_str(), index))
            .collect::<HashMap<_, _>>();
        let mut neighbors = vec![Vec::new(); document.nodes.len()];
        let mut edges = HashMap::new();
        for edge in &document.links {
            let (Some(&source), Some(&target)) = (
                positions.get(edge.source.as_str()),
                positions.get(edge.target.as_str()),
            ) else {
                continue;
            };
            if !neighbors[source].contains(&target) {
                neighbors[source].push(target);
            }
            edges.insert((source, target), edge);
            if !document.directed {
                if !neighbors[target].contains(&source) {
                    neighbors[target].push(source);
                }
                edges.insert((target, source), edge);
            }
        }
        Self {
            document,
            positions,
            neighbors,
            edges,
        }
    }

    fn query_tokens(&self, question: &str, depth: usize) -> usize {
        let terms = query_terms(question);
        let mut scored = self
            .document
            .nodes
            .iter()
            .filter_map(|node| {
                let label = node.label().to_lowercase();
                let score = terms.iter().filter(|term| label.contains(*term)).count();
                (score > 0).then_some((score, node.id.as_str()))
            })
            .collect::<Vec<_>>();
        scored.sort_by(|left, right| right.cmp(left));
        let starts = scored
            .into_iter()
            .take(3)
            .filter_map(|(_, id)| self.positions.get(id).copied())
            .collect::<HashSet<_>>();
        if starts.is_empty() {
            return 0;
        }
        let mut visited = starts.clone();
        let mut frontier = starts;
        let mut seen_edges = Vec::new();
        for _ in 0..depth {
            let mut next = HashSet::new();
            for node in &frontier {
                for &neighbor in &self.neighbors[*node] {
                    if !visited.contains(&neighbor) {
                        next.insert(neighbor);
                        seen_edges.push((*node, neighbor));
                    }
                }
            }
            visited.extend(next.iter().copied());
            frontier = next;
        }
        let mut length = 0;
        for node in &visited {
            let node = &self.document.nodes[*node];
            let source = node.string("source_file");
            let location = node.string("source_location");
            length += format!("NODE {} src={source} loc={location}", node.label())
                .chars()
                .count()
                + 1;
        }
        for (source, target) in seen_edges {
            if !visited.contains(&source) || !visited.contains(&target) {
                continue;
            }
            let relation = self
                .edges
                .get(&(source, target))
                .map(|edge| edge.string("relation"))
                .unwrap_or_default();
            length += format!(
                "EDGE {} --{relation}--> {}",
                self.document.nodes[source].label(),
                self.document.nodes[target].label()
            )
            .chars()
            .count()
                + 1;
        }
        (length.saturating_sub(1) / 4).max(1)
    }
}

fn round_one(value: f64) -> f64 {
    (value * 10.0).round() / 10.0
}

fn grouped(value: usize) -> String {
    let raw = value.to_string();
    raw.as_bytes()
        .rchunks(3)
        .rev()
        .map(|chunk| String::from_utf8_lossy(chunk))
        .collect::<Vec<_>>()
        .join(",")
}

fn python_float(value: f64) -> String {
    if value.fract() == 0.0 {
        format!("{value:.1}")
    } else {
        value.to_string()
    }
}
