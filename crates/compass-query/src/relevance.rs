//! Versioned, reviewed query-relevance fixtures and deterministic scoring.
//!
//! This module deliberately evaluates already-produced results.  It does not
//! participate in production ranking or query execution.

use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};
use thiserror::Error;

pub const QUERY_JUDGMENTS_SCHEMA_V1: &str = "compass.query-judgments/1";
pub const QUERY_QUALIFICATION_SCHEMA_V1: &str = "compass.query-qualification/1";
pub const MAX_QUESTIONS: usize = 256;
pub const MAX_JUDGMENTS_PER_QUERY: usize = 512;
pub const MAX_TEXT_BYTES: usize = 4 * 1024;
pub const MAX_LATENCY_MICROS: u64 = 3_600_000_000;

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum QueryClass {
    Exact,
    Lexical,
    Fuzzy,
    Intent,
    Edge,
    Path,
    Architecture,
    Negative,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct JudgmentCorpus {
    pub schema: String,
    pub corpus_id: String,
    pub graph_schema: String,
    pub graph_digest: String,
    pub repository_revision: String,
    pub analyzer_version: String,
    pub queries: Vec<JudgedQuery>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct JudgedQuery {
    pub id: String,
    pub text: String,
    pub class: QueryClass,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub locale: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expected_intent: Option<String>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub expected_slots: BTreeMap<String, String>,
    #[serde(default)]
    pub node_judgments: Vec<IdJudgment>,
    #[serde(default)]
    pub edge_judgments: Vec<EdgeJudgment>,
    #[serde(default)]
    pub path_judgments: Vec<PathJudgment>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub acceptable_ambiguity: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub must_not_return: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub notes: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct IdJudgment {
    pub id: String,
    pub grade: u8,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct EdgeIdentity {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub target: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub kind: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub direction: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct EdgeJudgment {
    pub edge: EdgeIdentity,
    pub grade: u8,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PathPattern {
    pub edge_kinds: Vec<String>,
    pub endpoint_ids: Vec<String>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PathJudgment {
    pub pattern: PathPattern,
    pub grade: u8,
}

#[derive(Clone, Debug, Error, Eq, PartialEq)]
pub enum RelevanceError {
    #[error("unsupported query-judgments schema {found}; expected {expected}")]
    UnsupportedSchema {
        found: String,
        expected: &'static str,
    },
    #[error("missing required field {field}")]
    MissingField { field: &'static str },
    #[error("{field} exceeds {limit} bytes")]
    TextTooLong { field: String, limit: usize },
    #[error("too many {field}: {actual}, maximum is {limit}")]
    LimitExceeded {
        field: &'static str,
        actual: usize,
        limit: usize,
    },
    #[error("duplicate {kind} id {id}")]
    DuplicateId { kind: &'static str, id: String },
    #[error("invalid relevance grade {grade}; grades must be in 0..=3")]
    InvalidGrade { grade: u8 },
    #[error("edge judgment must contain an id or source, target, kind, and direction")]
    InvalidEdgeIdentity,
    #[error("path judgment must contain at least one edge kind and two endpoints")]
    InvalidPathPattern,
    #[error("fixture graph digest {fixture} does not match graph digest {actual}")]
    GraphDigestMismatch { fixture: String, actual: String },
    #[error("metric {metric} is non-finite")]
    NonFiniteMetric { metric: String },
    #[error("duplicate observation for query id {id}")]
    DuplicateObservation { id: String },
    #[error("missing observation for corpus query id {id}")]
    MissingObservation { id: String },
    #[error("observation references unknown corpus query id {id}")]
    UnknownObservation { id: String },
    #[error("observation latency {micros}µs exceeds the {limit}µs qualification limit")]
    LatencyTooLarge { micros: u64, limit: u64 },
}

impl JudgmentCorpus {
    pub fn validate(&self) -> Result<(), RelevanceError> {
        if self.schema != QUERY_JUDGMENTS_SCHEMA_V1 {
            return Err(RelevanceError::UnsupportedSchema {
                found: self.schema.clone(),
                expected: QUERY_JUDGMENTS_SCHEMA_V1,
            });
        }
        for (field, value) in [
            ("corpusId", &self.corpus_id),
            ("graphSchema", &self.graph_schema),
            ("graphDigest", &self.graph_digest),
            ("repositoryRevision", &self.repository_revision),
            ("analyzerVersion", &self.analyzer_version),
        ] {
            require_text(field, value)?;
        }
        if self.queries.len() > MAX_QUESTIONS {
            return Err(RelevanceError::LimitExceeded {
                field: "queries",
                actual: self.queries.len(),
                limit: MAX_QUESTIONS,
            });
        }
        let mut ids = BTreeSet::new();
        for query in &self.queries {
            require_text("query.id", &query.id)?;
            require_text("query.text", &query.text)?;
            if !ids.insert(&query.id) {
                return Err(RelevanceError::DuplicateId {
                    kind: "query",
                    id: query.id.clone(),
                });
            }
            validate_query(query)?;
        }
        Ok(())
    }

    pub fn validate_graph_digest(&self, graph_digest: &str) -> Result<(), RelevanceError> {
        self.validate()?;
        if self.graph_digest != graph_digest {
            return Err(RelevanceError::GraphDigestMismatch {
                fixture: self.graph_digest.clone(),
                actual: graph_digest.to_owned(),
            });
        }
        Ok(())
    }

    #[must_use]
    pub fn stable_sorted(mut self) -> Self {
        self.queries.sort_by(|left, right| left.id.cmp(&right.id));
        for query in &mut self.queries {
            query
                .node_judgments
                .sort_by(|left, right| left.id.cmp(&right.id));
            query
                .edge_judgments
                .sort_by_key(|judgment| edge_key(&judgment.edge));
            query
                .path_judgments
                .sort_by_key(|judgment| path_key(&judgment.pattern));
            query.acceptable_ambiguity.sort();
            query.acceptable_ambiguity.dedup();
            query.must_not_return.sort();
            query.must_not_return.dedup();
        }
        self
    }
}

fn require_text(field: &str, value: &str) -> Result<(), RelevanceError> {
    if value.is_empty() {
        return Err(RelevanceError::MissingField {
            field: match field {
                "corpusId" => "corpusId",
                "graphSchema" => "graphSchema",
                "graphDigest" => "graphDigest",
                "repositoryRevision" => "repositoryRevision",
                "analyzerVersion" => "analyzerVersion",
                "query.id" => "query.id",
                "query.text" => "query.text",
                _ => "text",
            },
        });
    }
    if value.len() > MAX_TEXT_BYTES {
        return Err(RelevanceError::TextTooLong {
            field: field.to_owned(),
            limit: MAX_TEXT_BYTES,
        });
    }
    Ok(())
}

fn validate_query(query: &JudgedQuery) -> Result<(), RelevanceError> {
    let total =
        query.node_judgments.len() + query.edge_judgments.len() + query.path_judgments.len();
    if total > MAX_JUDGMENTS_PER_QUERY {
        return Err(RelevanceError::LimitExceeded {
            field: "judgments per query",
            actual: total,
            limit: MAX_JUDGMENTS_PER_QUERY,
        });
    }
    let mut node_ids = BTreeSet::new();
    for judgment in &query.node_judgments {
        require_text("node judgment id", &judgment.id)?;
        validate_grade(judgment.grade)?;
        if !node_ids.insert(&judgment.id) {
            return Err(RelevanceError::DuplicateId {
                kind: "node judgment",
                id: judgment.id.clone(),
            });
        }
    }
    let mut edge_ids = BTreeSet::new();
    for judgment in &query.edge_judgments {
        validate_grade(judgment.grade)?;
        let key = edge_key(&judgment.edge);
        if !edge_ids.insert(key.clone()) {
            return Err(RelevanceError::DuplicateId {
                kind: "edge judgment",
                id: key,
            });
        }
        let complete = judgment.edge.source.is_some()
            && judgment.edge.target.is_some()
            && judgment.edge.kind.is_some()
            && judgment.edge.direction.is_some();
        if judgment.edge.id.is_none() && !complete {
            return Err(RelevanceError::InvalidEdgeIdentity);
        }
    }
    let mut path_ids = BTreeSet::new();
    for judgment in &query.path_judgments {
        validate_grade(judgment.grade)?;
        if judgment.pattern.edge_kinds.is_empty() || judgment.pattern.endpoint_ids.len() < 2 {
            return Err(RelevanceError::InvalidPathPattern);
        }
        let key = path_key(&judgment.pattern);
        if !path_ids.insert(key.clone()) {
            return Err(RelevanceError::DuplicateId {
                kind: "path judgment",
                id: key,
            });
        }
    }
    validate_unique_ids("acceptable ambiguity", &query.acceptable_ambiguity)?;
    validate_unique_ids("must-not-return", &query.must_not_return)?;
    Ok(())
}

fn validate_unique_ids(kind: &'static str, values: &[String]) -> Result<(), RelevanceError> {
    let mut ids = BTreeSet::new();
    for value in values {
        require_text(kind, value)?;
        if !ids.insert(value) {
            return Err(RelevanceError::DuplicateId {
                kind,
                id: value.clone(),
            });
        }
    }
    Ok(())
}

fn validate_grade(grade: u8) -> Result<(), RelevanceError> {
    if grade > 3 {
        return Err(RelevanceError::InvalidGrade { grade });
    }
    Ok(())
}

fn edge_key(edge: &EdgeIdentity) -> String {
    edge.id.clone().unwrap_or_else(|| {
        format!(
            "{}|{}|{}|{}",
            edge.source.as_deref().unwrap_or_default(),
            edge.target.as_deref().unwrap_or_default(),
            edge.kind.as_deref().unwrap_or_default(),
            edge.direction.as_deref().unwrap_or_default(),
        )
    })
}

fn path_key(pattern: &PathPattern) -> String {
    format!(
        "{}|{}",
        pattern.edge_kinds.join(">"),
        pattern.endpoint_ids.join(">")
    )
}

#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct WorkCounts {
    pub candidates_read: u64,
    pub postings_decoded: u64,
    pub nodes_expanded: u64,
    pub edges_expanded: u64,
    pub response_bytes: u64,
}

#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ObservedEdge {
    pub id: String,
    pub source: String,
    pub target: String,
    pub kind: String,
    pub direction: String,
}

#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ObservedPath {
    pub edge_kinds: Vec<String>,
    pub endpoint_ids: Vec<String>,
}

#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct QueryObservation {
    pub query_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub intent: Option<String>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub slots: BTreeMap<String, String>,
    #[serde(default)]
    pub node_ids: Vec<String>,
    #[serde(default)]
    pub edges: Vec<ObservedEdge>,
    #[serde(default)]
    pub paths: Vec<ObservedPath>,
    #[serde(default)]
    pub no_answer: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub latency_micros: Option<u64>,
    #[serde(default)]
    pub work: WorkCounts,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct MetricValue {
    pub value: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub diagnostic: Option<String>,
}

impl MetricValue {
    fn defined(value: f64) -> Result<Self, RelevanceError> {
        if !value.is_finite() {
            return Err(RelevanceError::NonFiniteMetric {
                metric: "metric".to_owned(),
            });
        }
        Ok(Self {
            value: Some(value),
            diagnostic: None,
        })
    }

    fn undefined(diagnostic: &str) -> Self {
        Self {
            value: None,
            diagnostic: Some(diagnostic.to_owned()),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RelevanceMetrics {
    pub success_at_1: MetricValue,
    pub mrr_at_10: MetricValue,
    pub recall_at_5: MetricValue,
    pub recall_at_20: MetricValue,
    pub precision_at_10: MetricValue,
    pub ndcg_at_10: MetricValue,
    pub intent_macro_f1: MetricValue,
    pub entity_slot_exact_match: MetricValue,
    pub accepted_ambiguity_recall: MetricValue,
    pub edge_precision: MetricValue,
    pub edge_recall: MetricValue,
    pub edge_kind_precision: MetricValue,
    pub edge_kind_recall: MetricValue,
    pub edge_direction_precision: MetricValue,
    pub edge_direction_recall: MetricValue,
    pub path_acceptance_rate: MetricValue,
    pub mean_accepted_path_rank: MetricValue,
    pub no_answer_precision: MetricValue,
    pub false_positive_rate: MetricValue,
    pub latency_p50_micros: MetricValue,
    pub latency_p95_micros: MetricValue,
    pub per_intent: BTreeMap<String, IntentMetrics>,
    pub work: WorkCounts,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct IntentMetrics {
    pub precision: MetricValue,
    pub recall: MetricValue,
    pub f1: MetricValue,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct QualificationReport {
    pub schema: String,
    pub corpus_id: String,
    pub graph_digest: String,
    pub analyzer_version: String,
    pub ranker_version: String,
    pub planner_version: String,
    pub engine: String,
    pub limits: BTreeMap<String, u64>,
    pub metrics: RelevanceMetrics,
    pub diagnostics: Vec<String>,
}

pub fn score(
    corpus: &JudgmentCorpus,
    observations: &[QueryObservation],
) -> Result<RelevanceMetrics, RelevanceError> {
    corpus.validate()?;
    let corpus_ids = corpus
        .queries
        .iter()
        .map(|query| query.id.as_str())
        .collect::<BTreeSet<_>>();
    let mut observations_by_id = BTreeMap::new();
    for observation in observations {
        if !corpus_ids.contains(observation.query_id.as_str()) {
            return Err(RelevanceError::UnknownObservation {
                id: observation.query_id.clone(),
            });
        }
        if observation
            .latency_micros
            .is_some_and(|value| value > MAX_LATENCY_MICROS)
        {
            return Err(RelevanceError::LatencyTooLarge {
                micros: observation.latency_micros.unwrap_or_default(),
                limit: MAX_LATENCY_MICROS,
            });
        }
        if observations_by_id
            .insert(observation.query_id.as_str(), observation)
            .is_some()
        {
            return Err(RelevanceError::DuplicateObservation {
                id: observation.query_id.clone(),
            });
        }
    }
    for query in &corpus.queries {
        if !observations_by_id.contains_key(query.id.as_str()) {
            return Err(RelevanceError::MissingObservation {
                id: query.id.clone(),
            });
        }
    }
    let mut success = Ratio::default();
    let mut reciprocal = Ratio::default();
    let mut recall5 = Ratio::default();
    let mut recall20 = Ratio::default();
    let mut precision10 = Ratio::default();
    let mut ndcg10 = Ratio::default();
    let mut slots = Ratio::default();
    let mut ambiguity = Ratio::default();
    let mut edge_precision = Ratio::default();
    let mut edge_recall = Ratio::default();
    let mut edge_kind_precision = Ratio::default();
    let mut edge_kind_recall = Ratio::default();
    let mut edge_direction_precision = Ratio::default();
    let mut edge_direction_recall = Ratio::default();
    let mut paths = Ratio::default();
    let mut path_rank = Ratio::default();
    let mut no_answer_precision = Ratio::default();
    let mut false_positive = Ratio::default();
    let mut intent = BTreeMap::<String, IntentCounts>::new();
    let mut work = WorkCounts::default();
    let mut latency = Vec::new();

    for query in &corpus.queries {
        let observation = observations_by_id.get(query.id.as_str()).ok_or_else(|| {
            RelevanceError::MissingObservation {
                id: query.id.clone(),
            }
        })?;
        add_work(&mut work, &observation.work);
        if let Some(value) = observation.latency_micros {
            latency.push(value);
        }
        score_nodes(
            query,
            observation,
            &mut success,
            &mut reciprocal,
            &mut recall5,
            &mut recall20,
            &mut precision10,
            &mut ndcg10,
            &mut ambiguity,
        );
        score_slots(query, observation, &mut slots);
        score_edges(
            query,
            observation,
            &mut edge_precision,
            &mut edge_recall,
            &mut edge_kind_precision,
            &mut edge_kind_recall,
            &mut edge_direction_precision,
            &mut edge_direction_recall,
        );
        score_paths(query, observation, &mut paths, &mut path_rank);
        score_no_answer(
            query,
            observation,
            &mut no_answer_precision,
            &mut false_positive,
        );
        score_intent(query, observation, &mut intent);
    }
    latency.sort_unstable();

    let mut per_intent = BTreeMap::new();
    let mut macro_f1 = Ratio::default();
    for (name, counts) in intent {
        let precision = ratio(
            counts.true_positive,
            counts.predicted,
            "no predicted intent instances",
        )?;
        let recall = ratio(
            counts.true_positive,
            counts.expected,
            "no expected intent instances",
        )?;
        let f1 = match (precision.value, recall.value) {
            (Some(precision), Some(recall)) if precision + recall > 0.0 => {
                MetricValue::defined(2.0 * precision * recall / (precision + recall))?
            }
            (Some(_), Some(_)) => MetricValue::defined(0.0)?,
            _ => MetricValue::undefined("intent precision or recall is undefined"),
        };
        if let Some(value) = f1.value {
            macro_f1.add(value, 1.0);
        }
        per_intent.insert(
            name,
            IntentMetrics {
                precision,
                recall,
                f1,
            },
        );
    }
    Ok(RelevanceMetrics {
        success_at_1: success.metric("no exact/entity judgments")?,
        mrr_at_10: reciprocal.metric("no exact/entity judgments")?,
        recall_at_5: recall5.metric("no relevant node judgments")?,
        recall_at_20: recall20.metric("no relevant node judgments")?,
        precision_at_10: precision10.metric("no returned ranked nodes")?,
        ndcg_at_10: ndcg10.metric("no graded node judgments")?,
        intent_macro_f1: macro_f1.metric("no intent labels")?,
        entity_slot_exact_match: slots.metric("no expected slots")?,
        accepted_ambiguity_recall: ambiguity.metric("no accepted ambiguity alternatives")?,
        edge_precision: edge_precision.metric("no returned edges")?,
        edge_recall: edge_recall.metric("no relevant edge judgments")?,
        edge_kind_precision: edge_kind_precision.metric("no returned edge kinds")?,
        edge_kind_recall: edge_kind_recall.metric("no judged edge kinds")?,
        edge_direction_precision: edge_direction_precision
            .metric("no returned edge directions with direction judgments")?,
        edge_direction_recall: edge_direction_recall.metric("no judged edge directions")?,
        path_acceptance_rate: paths.metric("no path judgments")?,
        mean_accepted_path_rank: path_rank.metric("no accepted returned paths")?,
        no_answer_precision: no_answer_precision.metric("no predicted no-answer results")?,
        false_positive_rate: false_positive.metric("no negative judgments")?,
        latency_p50_micros: percentile_metric(&latency, 50, "no latency observations")?,
        latency_p95_micros: percentile_metric(&latency, 95, "no latency observations")?,
        per_intent,
        work,
    })
}

pub fn qualification_report(
    corpus: &JudgmentCorpus,
    observations: &[QueryObservation],
    ranker_version: &str,
    planner_version: &str,
    engine: &str,
    limits: BTreeMap<String, u64>,
) -> Result<QualificationReport, RelevanceError> {
    let mut diagnostics = Vec::new();
    let metrics = score(corpus, observations)?;
    for (name, metric) in metric_entries(&metrics) {
        if let Some(diagnostic) = &metric.diagnostic {
            diagnostics.push(format!("{name}: {diagnostic}"));
        }
    }
    diagnostics.sort();
    Ok(QualificationReport {
        schema: QUERY_QUALIFICATION_SCHEMA_V1.to_owned(),
        corpus_id: corpus.corpus_id.clone(),
        graph_digest: corpus.graph_digest.clone(),
        analyzer_version: corpus.analyzer_version.clone(),
        ranker_version: ranker_version.to_owned(),
        planner_version: planner_version.to_owned(),
        engine: engine.to_owned(),
        limits,
        metrics,
        diagnostics,
    })
}

fn metric_entries(metrics: &RelevanceMetrics) -> [(&'static str, &MetricValue); 21] {
    [
        ("successAt1", &metrics.success_at_1),
        ("mrrAt10", &metrics.mrr_at_10),
        ("recallAt5", &metrics.recall_at_5),
        ("recallAt20", &metrics.recall_at_20),
        ("precisionAt10", &metrics.precision_at_10),
        ("ndcgAt10", &metrics.ndcg_at_10),
        ("intentMacroF1", &metrics.intent_macro_f1),
        ("entitySlotExactMatch", &metrics.entity_slot_exact_match),
        (
            "acceptedAmbiguityRecall",
            &metrics.accepted_ambiguity_recall,
        ),
        ("edgePrecision", &metrics.edge_precision),
        ("edgeRecall", &metrics.edge_recall),
        ("edgeKindPrecision", &metrics.edge_kind_precision),
        ("edgeKindRecall", &metrics.edge_kind_recall),
        ("edgeDirectionPrecision", &metrics.edge_direction_precision),
        ("edgeDirectionRecall", &metrics.edge_direction_recall),
        ("pathAcceptanceRate", &metrics.path_acceptance_rate),
        ("meanAcceptedPathRank", &metrics.mean_accepted_path_rank),
        ("noAnswerPrecision", &metrics.no_answer_precision),
        ("falsePositiveRate", &metrics.false_positive_rate),
        ("latencyP50Micros", &metrics.latency_p50_micros),
        ("latencyP95Micros", &metrics.latency_p95_micros),
    ]
}

#[derive(Default)]
struct Ratio {
    numerator: f64,
    denominator: f64,
}
impl Ratio {
    fn add(&mut self, numerator: f64, denominator: f64) {
        self.numerator += numerator;
        self.denominator += denominator;
    }
    fn metric(&self, diagnostic: &str) -> Result<MetricValue, RelevanceError> {
        ratio_f64(self.numerator, self.denominator, diagnostic)
    }
}
fn ratio(
    numerator: u64,
    denominator: u64,
    diagnostic: &str,
) -> Result<MetricValue, RelevanceError> {
    ratio_f64(numerator as f64, denominator as f64, diagnostic)
}
fn ratio_f64(
    numerator: f64,
    denominator: f64,
    diagnostic: &str,
) -> Result<MetricValue, RelevanceError> {
    if denominator == 0.0 {
        Ok(MetricValue::undefined(diagnostic))
    } else {
        MetricValue::defined(numerator / denominator)
    }
}

fn percentile_metric(
    sorted: &[u64],
    percentile: usize,
    diagnostic: &str,
) -> Result<MetricValue, RelevanceError> {
    let Some(last) = sorted.len().checked_sub(1) else {
        return Ok(MetricValue::undefined(diagnostic));
    };
    let rank = (percentile * sorted.len()).div_ceil(100).saturating_sub(1);
    MetricValue::defined(sorted[rank.min(last)] as f64)
}
#[derive(Default)]
struct IntentCounts {
    true_positive: u64,
    predicted: u64,
    expected: u64,
}

// These private helpers update distinct named metric accumulators. Keeping the
// references explicit makes the metric-to-denominator mapping auditable.
#[allow(clippy::too_many_arguments)]
fn score_nodes(
    query: &JudgedQuery,
    observation: &QueryObservation,
    success: &mut Ratio,
    reciprocal: &mut Ratio,
    recall5: &mut Ratio,
    recall20: &mut Ratio,
    precision10: &mut Ratio,
    ndcg10: &mut Ratio,
    ambiguity: &mut Ratio,
) {
    let grades = query
        .node_judgments
        .iter()
        .map(|item| (&item.id, item.grade))
        .collect::<BTreeMap<_, _>>();
    let exact = grades
        .iter()
        .filter_map(|(id, grade)| (*grade == 3).then_some(*id))
        .collect::<BTreeSet<_>>();
    if !exact.is_empty() {
        success.add(
            observation
                .node_ids
                .first()
                .is_some_and(|id| exact.contains(id)) as u8 as f64,
            1.0,
        );
        let reciprocal_rank = observation
            .node_ids
            .iter()
            .take(10)
            .position(|id| exact.contains(id))
            .map_or(0.0, |position| 1.0 / (position + 1) as f64);
        reciprocal.add(reciprocal_rank, 1.0);
    }
    let relevant = grades
        .iter()
        .filter_map(|(id, grade)| (*grade >= 2).then_some(*id))
        .collect::<BTreeSet<_>>();
    if !relevant.is_empty() {
        let found5 = observation
            .node_ids
            .iter()
            .take(5)
            .filter(|id| relevant.contains(id))
            .collect::<BTreeSet<_>>()
            .len();
        let found20 = observation
            .node_ids
            .iter()
            .take(20)
            .filter(|id| relevant.contains(id))
            .collect::<BTreeSet<_>>()
            .len();
        recall5.add(found5 as f64, relevant.len() as f64);
        recall20.add(found20 as f64, relevant.len() as f64);
    }
    let returned = observation.node_ids.iter().take(10).collect::<Vec<_>>();
    if !returned.is_empty() {
        let useful = returned
            .iter()
            .filter(|id| grades.get(**id).copied().unwrap_or(0) >= 2)
            .count();
        precision10.add(useful as f64, returned.len() as f64);
    }
    if !grades.is_empty() {
        let dcg = returned
            .iter()
            .enumerate()
            .map(|(index, id)| {
                gain(grades.get(*id).copied().unwrap_or(0)) / ((index + 2) as f64).log2()
            })
            .sum::<f64>();
        let mut ideal = grades.values().copied().collect::<Vec<_>>();
        ideal.sort_by(|left, right| right.cmp(left));
        let ideal_dcg = ideal
            .into_iter()
            .take(10)
            .enumerate()
            .map(|(index, grade)| gain(grade) / ((index + 2) as f64).log2())
            .sum::<f64>();
        if ideal_dcg > 0.0 {
            ndcg10.add(dcg / ideal_dcg, 1.0);
        }
    }
    if !query.acceptable_ambiguity.is_empty() {
        let acceptable = query.acceptable_ambiguity.iter().collect::<BTreeSet<_>>();
        let found = observation
            .node_ids
            .iter()
            .filter(|id| acceptable.contains(id))
            .collect::<BTreeSet<_>>()
            .len();
        ambiguity.add(found as f64, acceptable.len() as f64);
    }
}
fn gain(grade: u8) -> f64 {
    (1_u32 << u32::from(grade)).saturating_sub(1) as f64
}
fn score_slots(query: &JudgedQuery, observation: &QueryObservation, slots: &mut Ratio) {
    if !query.expected_slots.is_empty() {
        slots.add(
            (query.expected_slots == observation.slots) as u8 as f64,
            1.0,
        );
    }
}
// Edge identity, kind, and direction are intentionally accumulated separately
// so an id-only judgment cannot imply direction correctness.
#[allow(clippy::too_many_arguments)]
fn score_edges(
    query: &JudgedQuery,
    observation: &QueryObservation,
    precision: &mut Ratio,
    recall: &mut Ratio,
    kind_precision: &mut Ratio,
    kind_recall: &mut Ratio,
    direction_precision: &mut Ratio,
    direction_recall: &mut Ratio,
) {
    let relevant = query
        .edge_judgments
        .iter()
        .filter(|item| item.grade >= 2)
        .map(|item| &item.edge)
        .collect::<Vec<_>>();
    if !observation.edges.is_empty() {
        let correct = observation
            .edges
            .iter()
            .filter(|edge| relevant.iter().any(|expected| edge_matches(expected, edge)))
            .count();
        precision.add(correct as f64, observation.edges.len() as f64);
    }
    if !relevant.is_empty() {
        let found = relevant
            .iter()
            .filter(|expected| {
                observation
                    .edges
                    .iter()
                    .any(|edge| edge_matches(expected, edge))
            })
            .count();
        recall.add(found as f64, relevant.len() as f64);
    }
    score_edge_dimension(
        relevant.iter().filter_map(|edge| edge.kind.as_deref()),
        observation.edges.iter().map(|edge| edge.kind.as_str()),
        kind_precision,
        kind_recall,
        "kind",
    );
    score_edge_dimension(
        relevant.iter().filter_map(|edge| edge.direction.as_deref()),
        observation.edges.iter().map(|edge| edge.direction.as_str()),
        direction_precision,
        direction_recall,
        "direction",
    );
}
fn score_edge_dimension<'a>(
    expected: impl Iterator<Item = &'a str>,
    returned: impl Iterator<Item = &'a str>,
    precision: &mut Ratio,
    recall: &mut Ratio,
    _dimension: &str,
) {
    let expected = expected.collect::<BTreeSet<_>>();
    if expected.is_empty() {
        return;
    }
    let returned = returned.collect::<BTreeSet<_>>();
    if !returned.is_empty() {
        precision.add(
            returned.intersection(&expected).count() as f64,
            returned.len() as f64,
        );
    }
    recall.add(
        returned.intersection(&expected).count() as f64,
        expected.len() as f64,
    );
}

fn edge_matches(expected: &EdgeIdentity, observed: &ObservedEdge) -> bool {
    if let Some(id) = &expected.id {
        return id == &observed.id;
    }
    expected.source.as_deref() == Some(observed.source.as_str())
        && expected.target.as_deref() == Some(observed.target.as_str())
        && expected.kind.as_deref() == Some(observed.kind.as_str())
        && expected.direction.as_deref() == Some(observed.direction.as_str())
}
fn score_paths(
    query: &JudgedQuery,
    observation: &QueryObservation,
    acceptance: &mut Ratio,
    rank: &mut Ratio,
) {
    let accepted = query
        .path_judgments
        .iter()
        .filter(|item| item.grade >= 2)
        .map(|item| path_key(&item.pattern))
        .collect::<BTreeSet<_>>();
    if accepted.is_empty() {
        return;
    }
    let found = observation.paths.iter().position(|path| {
        accepted.contains(&format!(
            "{}|{}",
            path.edge_kinds.join(">"),
            path.endpoint_ids.join(">")
        ))
    });
    acceptance.add(found.is_some() as u8 as f64, 1.0);
    if let Some(position) = found {
        rank.add((position + 1) as f64, 1.0);
    }
}
fn score_no_answer(
    query: &JudgedQuery,
    observation: &QueryObservation,
    precision: &mut Ratio,
    false_positive: &mut Ratio,
) {
    if observation.no_answer {
        precision.add(
            matches!(query.class, QueryClass::Negative) as u8 as f64,
            1.0,
        );
    }
    if !matches!(query.class, QueryClass::Negative) {
        return;
    }
    if !observation.no_answer
        || !observation.node_ids.is_empty()
        || !observation.edges.is_empty()
        || observation
            .node_ids
            .iter()
            .any(|id| query.must_not_return.contains(id))
    {
        false_positive.add(1.0, 1.0);
    } else {
        false_positive.add(0.0, 1.0);
    }
}
fn score_intent(
    query: &JudgedQuery,
    observation: &QueryObservation,
    counts: &mut BTreeMap<String, IntentCounts>,
) {
    if let Some(expected) = &query.expected_intent {
        counts.entry(expected.clone()).or_default().expected += 1;
    }
    if let Some(predicted) = &observation.intent {
        counts.entry(predicted.clone()).or_default().predicted += 1;
    }
    if query.expected_intent.is_some()
        && query.expected_intent == observation.intent
        && let Some(intent) = &query.expected_intent
    {
        counts.entry(intent.clone()).or_default().true_positive += 1;
    }
}
fn add_work(total: &mut WorkCounts, work: &WorkCounts) {
    total.candidates_read += work.candidates_read;
    total.postings_decoded += work.postings_decoded;
    total.nodes_expanded += work.nodes_expanded;
    total.edges_expanded += work.edges_expanded;
    total.response_bytes += work.response_bytes;
}

#[cfg(test)]
mod tests {
    use super::*;

    fn corpus() -> JudgmentCorpus {
        JudgmentCorpus {
            schema: QUERY_JUDGMENTS_SCHEMA_V1.to_owned(),
            corpus_id: "fixture".to_owned(),
            graph_schema: "compass.graph/1".to_owned(),
            graph_digest: "sha256:fixture".to_owned(),
            repository_revision: "deadbeef".to_owned(),
            analyzer_version: "test".to_owned(),
            queries: vec![JudgedQuery {
                id: "q1".to_owned(),
                text: "find user".to_owned(),
                class: QueryClass::Exact,
                locale: None,
                expected_intent: Some("search".to_owned()),
                expected_slots: BTreeMap::new(),
                node_judgments: vec![
                    IdJudgment {
                        id: "n:exact".to_owned(),
                        grade: 3,
                    },
                    IdJudgment {
                        id: "n:relevant".to_owned(),
                        grade: 2,
                    },
                    IdJudgment {
                        id: "n:context".to_owned(),
                        grade: 1,
                    },
                ],
                edge_judgments: Vec::new(),
                path_judgments: Vec::new(),
                acceptable_ambiguity: vec!["n:exact".to_owned()],
                must_not_return: Vec::new(),
                notes: None,
            }],
        }
    }
    #[test]
    fn relevance_contract_round_trips_and_rejects_invalid_inputs() -> Result<(), RelevanceError> {
        let fixture = corpus();
        fixture.validate_graph_digest("sha256:fixture")?;
        let encoded =
            serde_json::to_string(&fixture).map_err(|_| RelevanceError::MissingField {
                field: "serialization",
            })?;
        let decoded: JudgmentCorpus =
            serde_json::from_str(&encoded).map_err(|_| RelevanceError::MissingField {
                field: "deserialization",
            })?;
        assert_eq!(fixture, decoded);
        let mut invalid = fixture.clone();
        invalid.schema = "compass.query-judgments/2".to_owned();
        assert!(matches!(
            invalid.validate(),
            Err(RelevanceError::UnsupportedSchema { .. })
        ));
        let mut invalid = fixture.clone();
        invalid.queries[0].node_judgments[0].grade = 4;
        assert!(matches!(
            invalid.validate(),
            Err(RelevanceError::InvalidGrade { .. })
        ));
        let mut invalid = fixture.clone();
        invalid.queries.push(invalid.queries[0].clone());
        assert!(matches!(
            invalid.validate(),
            Err(RelevanceError::DuplicateId { .. })
        ));
        let mut invalid = fixture.clone();
        invalid.queries[0].text = "x".repeat(MAX_TEXT_BYTES + 1);
        assert!(matches!(
            invalid.validate(),
            Err(RelevanceError::TextTooLong { .. })
        ));
        assert!(matches!(
            fixture.validate_graph_digest("sha256:other"),
            Err(RelevanceError::GraphDigestMismatch { .. })
        ));
        Ok(())
    }
    #[test]
    fn relevance_metrics_are_exact_and_finite() -> Result<(), RelevanceError> {
        let fixture = corpus();
        let report = qualification_report(
            &fixture,
            &[QueryObservation {
                query_id: "q1".to_owned(),
                intent: Some("search".to_owned()),
                slots: BTreeMap::new(),
                node_ids: vec![
                    "n:exact".to_owned(),
                    "n:relevant".to_owned(),
                    "n:context".to_owned(),
                ],
                edges: Vec::new(),
                paths: Vec::new(),
                no_answer: false,
                latency_micros: Some(10),
                work: WorkCounts {
                    candidates_read: 2,
                    ..WorkCounts::default()
                },
            }],
            "ranker",
            "planner",
            "json",
            BTreeMap::new(),
        )?;
        assert_eq!(report.metrics.success_at_1.value, Some(1.0));
        assert_eq!(report.metrics.recall_at_5.value, Some(1.0));
        assert_eq!(report.metrics.ndcg_at_10.value, Some(1.0));
        assert!(
            !serde_json::to_string(&report)
                .map_err(|_| RelevanceError::MissingField {
                    field: "serialization"
                })?
                .contains("NaN")
        );
        Ok(())
    }

    #[test]
    fn relevance_metrics_cover_edges_paths_ambiguity_and_no_answer() -> Result<(), RelevanceError> {
        let mut fixture = corpus();
        fixture.queries.push(JudgedQuery {
            id: "q2".to_owned(),
            text: "who calls list".to_owned(),
            class: QueryClass::Path,
            locale: None,
            expected_intent: Some("callers".to_owned()),
            expected_slots: BTreeMap::from([("direction".to_owned(), "incoming".to_owned())]),
            node_judgments: Vec::new(),
            edge_judgments: vec![EdgeJudgment {
                edge: EdgeIdentity {
                    id: Some("e:calls".to_owned()),
                    source: None,
                    target: None,
                    kind: None,
                    direction: None,
                },
                grade: 3,
            }],
            path_judgments: vec![PathJudgment {
                pattern: PathPattern {
                    edge_kinds: vec!["calls".to_owned()],
                    endpoint_ids: vec!["n:caller".to_owned(), "n:exact".to_owned()],
                },
                grade: 3,
            }],
            acceptable_ambiguity: Vec::new(),
            must_not_return: Vec::new(),
            notes: None,
        });
        fixture.queries.push(JudgedQuery {
            id: "q3".to_owned(),
            text: "missing symbol".to_owned(),
            class: QueryClass::Negative,
            locale: None,
            expected_intent: Some("search".to_owned()),
            expected_slots: BTreeMap::new(),
            node_judgments: Vec::new(),
            edge_judgments: Vec::new(),
            path_judgments: Vec::new(),
            acceptable_ambiguity: Vec::new(),
            must_not_return: vec!["n:false-positive".to_owned()],
            notes: Some("Reviewed as a legitimate no-answer query.".to_owned()),
        });
        let metrics = score(
            &fixture,
            &[
                QueryObservation {
                    query_id: "q1".to_owned(),
                    intent: Some("search".to_owned()),
                    slots: BTreeMap::new(),
                    node_ids: vec!["n:relevant".to_owned(), "n:exact".to_owned()],
                    edges: Vec::new(),
                    paths: Vec::new(),
                    no_answer: false,
                    latency_micros: Some(20),
                    work: WorkCounts::default(),
                },
                QueryObservation {
                    query_id: "q2".to_owned(),
                    intent: Some("callers".to_owned()),
                    slots: BTreeMap::from([("direction".to_owned(), "incoming".to_owned())]),
                    node_ids: Vec::new(),
                    edges: vec![ObservedEdge {
                        id: "e:calls".to_owned(),
                        source: "n:caller".to_owned(),
                        target: "n:exact".to_owned(),
                        kind: "calls".to_owned(),
                        direction: "outgoing".to_owned(),
                    }],
                    paths: vec![ObservedPath {
                        edge_kinds: vec!["calls".to_owned()],
                        endpoint_ids: vec!["n:caller".to_owned(), "n:exact".to_owned()],
                    }],
                    no_answer: false,
                    latency_micros: Some(30),
                    work: WorkCounts::default(),
                },
                QueryObservation {
                    query_id: "q3".to_owned(),
                    intent: Some("search".to_owned()),
                    slots: BTreeMap::new(),
                    node_ids: Vec::new(),
                    edges: Vec::new(),
                    paths: Vec::new(),
                    no_answer: true,
                    latency_micros: Some(40),
                    work: WorkCounts::default(),
                },
            ],
        )?;
        assert_eq!(metrics.success_at_1.value, Some(0.0));
        assert_eq!(metrics.mrr_at_10.value, Some(0.5));
        assert_eq!(metrics.edge_precision.value, Some(1.0));
        assert_eq!(metrics.path_acceptance_rate.value, Some(1.0));
        assert_eq!(metrics.mean_accepted_path_rank.value, Some(1.0));
        assert_eq!(metrics.no_answer_precision.value, Some(1.0));
        assert_eq!(metrics.false_positive_rate.value, Some(0.0));
        assert_eq!(metrics.entity_slot_exact_match.value, Some(1.0));
        Ok(())
    }

    #[test]
    fn undefined_metrics_are_null_with_diagnostics() -> Result<(), RelevanceError> {
        let mut fixture = corpus();
        fixture.queries[0].node_judgments.clear();
        fixture.queries[0].acceptable_ambiguity.clear();
        fixture.queries[0].expected_intent = None;
        let report = qualification_report(
            &fixture,
            &[QueryObservation {
                query_id: "q1".to_owned(),
                latency_micros: None,
                ..QueryObservation::default()
            }],
            "ranker",
            "planner",
            "json",
            BTreeMap::new(),
        )?;
        assert_eq!(report.metrics.ndcg_at_10.value, None);
        assert!(report.metrics.ndcg_at_10.diagnostic.is_some());
        let encoded = serde_json::to_string(&report).map_err(|_| RelevanceError::MissingField {
            field: "serialization",
        })?;
        assert!(encoded.contains("\"value\":null"));
        Ok(())
    }

    #[test]
    fn observations_are_complete_unique_and_measure_direction_separately()
    -> Result<(), RelevanceError> {
        let mut fixture = corpus();
        fixture.queries[0].edge_judgments.push(EdgeJudgment {
            edge: EdgeIdentity {
                id: None,
                source: Some("n:caller".to_owned()),
                target: Some("n:exact".to_owned()),
                kind: Some("calls".to_owned()),
                direction: Some("outgoing".to_owned()),
            },
            grade: 3,
        });
        let observation = QueryObservation {
            query_id: "q1".to_owned(),
            edges: vec![ObservedEdge {
                id: "e:calls".to_owned(),
                source: "n:caller".to_owned(),
                target: "n:exact".to_owned(),
                kind: "calls".to_owned(),
                direction: "incoming".to_owned(),
            }],
            ..QueryObservation::default()
        };
        assert!(matches!(
            score(&fixture, &[]),
            Err(RelevanceError::MissingObservation { .. })
        ));
        assert!(matches!(
            score(&fixture, &[observation.clone(), observation.clone()]),
            Err(RelevanceError::DuplicateObservation { .. })
        ));
        let metrics = score(&fixture, &[observation])?;
        assert_eq!(metrics.edge_kind_precision.value, Some(1.0));
        assert_eq!(metrics.edge_direction_precision.value, Some(0.0));
        assert_eq!(metrics.edge_direction_recall.value, Some(0.0));
        Ok(())
    }
}
